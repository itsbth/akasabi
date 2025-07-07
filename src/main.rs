#![deny(clippy::all)]
#![deny(clippy::pedantic)]
use crate::indexer::create_index;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use etcetera::choose_app_strategy;
use etcetera::AppStrategy;
use etcetera::AppStrategyArgs;
use itertools::izip;
use itertools::Itertools;
use std::clone::Clone;
use std::fs::create_dir_all;
use std::path::PathBuf;
use tantivy::schema::Schema;
use tantivy::{DocAddress, Document, Index, Score, Searcher};
use yansi::{Color, Paint, Style};

mod config;
mod indexer;

#[derive(clap::ValueEnum, Clone)]
enum Field {
    Word,
    Reading,
    ReadingRomaji,
    Meaning,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum ColorArg {
    Auto,
    Always,
    Never,
}

#[derive(Parser)]
#[command(author, version, about)]
struct Args {
    #[clap(short, long, global = true)]
    config: Option<PathBuf>,
    #[clap(short, long, global = true, env = "AKASABI_INDEX")]
    index: Option<PathBuf>,
    #[clap(long, global = true, default_value = "auto")]
    color: ColorArg,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Search {
        term: String,
        #[clap(short, long)]
        field: Option<Field>,
        #[clap(short = 'm', long)]
        create_if_missing: bool,
    },
    Index {
        #[clap(
            short,
            long,
            help = "Path to JMdict.gz file",
            default_value = "JMdict_e.gz"
        )]
        path: String,
        #[clap(
            short,
            long,
            help = "Automatically download the latest JMdict.gz file if it doesn't exist"
        )]
        jmdict_url: Option<String>,
    },
    Info,
    // Primarily for debugging
    PrintConfig {
        // index is already a global option
        #[clap(long)]
        jmdict_url: Option<String>,
        #[clap(long)]
        jmdict_path: Option<PathBuf>,
    },
}

// TODO: Refactor this into multiple functions
#[allow(clippy::too_many_lines)]
fn main() -> Result<()> {
    env_logger::init();

    let args = Args::parse();
    let strategy = choose_app_strategy(AppStrategyArgs {
        app_name: "akasabi".to_string(),
        top_level_domain: "com".to_string(),
        author: "itsbth".to_string(),
    })?;

    let config = {
        let config_path = args
            .config
            .clone()
            .unwrap_or_else(|| strategy.in_config_dir("config.toml"));

        if config_path.exists() {
            config::Config::from_file(&config_path)?
        } else {
            config::Config::default()
        }
    };

    {
        let color = match args.color {
            ColorArg::Auto => {
                nix::unistd::isatty(nix::libc::STDOUT_FILENO).unwrap_or(false)
                    && !std::env::var("NO_COLOR")
                        .map(|s| s.is_empty())
                        .unwrap_or(true)
            }
            ColorArg::Always => true,
            ColorArg::Never => false,
        };

        if color {
            Paint::enable_windows_ascii();
            Paint::enable();
        } else {
            Paint::disable();
        }
    }

    let config_index_path = config.index.path.as_ref();

    // Try --index, config file, then default
    let index_path = args
        .index
        .as_ref()
        .or(config_index_path)
        .map_or_else(|| strategy.in_data_dir("index"), Clone::clone);

    let schema = indexer::create_schema();

    let index = if index_path.join("meta.json").exists() {
        Index::open_in_dir(&index_path).context("Failed to open index")?
    } else {
        create_dir_all(index_path.clone()).context("Failed to create index directory")?;
        Index::create_in_dir(&index_path, schema.clone()).context("Failed to create index")?
    };

    match args.command {
        Command::Search {
            term,
            field,
            create_if_missing: _,
        } => {
            let (searcher, top_docs) = search(&index, &schema, &term, field.as_ref())?;

            for (_score, doc_address) in top_docs {
                let retrieved_doc = searcher.doc(doc_address)?;
                print_result(&schema, &retrieved_doc, &term);
            }
        }
        Command::Index { path, .. } => {
            index_(&index, &schema, &path)?;
        }
        Command::Info => {
            // Print program info; ie version and configuration (currently only resolved index path)
            println!(
                "akasabi {}{}",
                Paint::green("v"),
                Paint::green(env!("CARGO_PKG_VERSION"))
            );
            println!("Index path: {}", Paint::blue(index_path.to_str().unwrap()));
            {
                // FIXME: This is duplicated from above
                let config_path = args
                    .config
                    .unwrap_or_else(|| strategy.in_config_dir("config.toml"));
                println!(
                    "Config path: {}",
                    Paint::blue(config_path.to_str().unwrap())
                );
                println!("Config: {config:#?}");
            }
        }
        Command::PrintConfig {
            jmdict_url,
            jmdict_path,
        } => {
            let string = config::Config {
                index: config::Index {
                    path: Some(index_path),
                },
                jmdict: config::Jmdict {
                    path: jmdict_path.or(config.jmdict.path),
                    url: jmdict_url.or(config.jmdict.url),
                },
            }
            .to_str()?;
            print!("{string}");
        }
    }

    Ok(())
}

fn index_(index: &Index, schema: &Schema, path: &str) -> Result<()> {
    create_index(schema, path, index)?;
    Ok(())
}

fn search(
    index: &Index,
    schema: &Schema,
    term: &str,
    field: Option<&Field>,
) -> Result<(Searcher, Vec<(Score, DocAddress)>)> {
    let (word, reading, reading_romaji, meaning) = (
        schema.get_field("word").unwrap(),
        schema.get_field("reading").unwrap(),
        schema.get_field("reading_romaji").unwrap(),
        schema.get_field("meaning").unwrap(),
    );

    let reader = index
        .reader_builder()
        .reload_policy(tantivy::ReloadPolicy::Manual)
        .try_into()?;
    let searcher = reader.searcher();

    let fields = match field {
        Some(&Field::Word) => vec![word],
        Some(&Field::Reading) => vec![reading],
        Some(&Field::ReadingRomaji) => vec![reading_romaji],
        Some(&Field::Meaning) => vec![meaning],
        None => vec![word, reading, reading_romaji, meaning],
    };

    let mut query_parser = tantivy::query::QueryParser::for_index(index, fields);
    query_parser.set_conjunction_by_default();

    let query = query_parser.parse_query(term)?;

    let top_docs = searcher.search(&query, &tantivy::collector::TopDocs::with_limit(10))?;

    Ok((searcher, top_docs))
}

// TODO: Also take query so we can highlight it
fn print_result(schema: &Schema, document: &Document, _term: &str) {
    // entry fields
    let word = schema.get_field("word").unwrap();
    let reading = schema.get_field("reading").unwrap();
    // let reading_romaji = schema.get_field("reading_romaji").unwrap();

    // sense fields
    let meaning = schema.get_field("meaning").unwrap();
    let pos = schema.get_field("pos").unwrap();
    let field = schema.get_field("field").unwrap();

    // myougiden format:
    // kanji [;kanji]* (reading [、reading]*)*
    // 1. \[poc\] meaning [; meaning]*
    // 2. \[field\] meaning [; meaning]*

    let kanji = document
        .get_all(word)
        .map(|f| f.as_text().unwrap())
        .collect_vec();
    let readings = document
        .get_all(reading)
        .map(|f| f.as_text().unwrap())
        .collect_vec();

    // meanings, pos, and fields should be "aligned" (ie. same length, n-th element of each)
    let meanings = document
        .get_all(meaning)
        .map(|f| f.as_text().unwrap())
        .collect_vec();
    let pos = document
        .get_all(pos)
        .map(|f| f.as_text().unwrap())
        .collect_vec();
    let fields = document
        .get_all(field)
        .map(|f| f.as_text().unwrap())
        .collect_vec();

    let c_kanji = Style::new(Color::Blue).bold();
    let c_reading = Style::new(Color::Magenta).bold();

    // field shares style with pos
    let c_pos = Style::new(Color::Yellow).bold();
    let c_meaning = Style::new(Color::Default).bold();
    // let c_highlight = Style::new(Color::Red).bold();
    let c_index = Style::new(Color::Green).bold();

    if kanji.is_empty() {
        println!("{}", c_kanji.paint(readings.join("、")));
    } else {
        println!(
            "{} ({})",
            // TODO: Style separator separately
            c_kanji.paint(kanji.join("; ")),
            c_reading.paint(readings.join("、"))
        );
    }

    for (idx, (meaning, pos, field)) in izip!(meanings, pos, fields).enumerate() {
        let meanings = meaning.split("; ").collect_vec();

        // TODO: Properly handle pos and field (split and re-join)
        print!(
            "{} [{};{}]",
            c_index.paint(format!("{}.", idx + 1)),
            c_pos.paint(pos),
            c_pos.paint(field)
        );
        for (idx, meaning) in meanings.iter().enumerate() {
            if idx == 0 {
                print!(" {}", c_meaning.paint(meaning));
                continue;
            }
            print!("{}{}", Paint::yellow("; "), c_meaning.paint(meaning));
        }
        println!();
    }
    println!();
}
