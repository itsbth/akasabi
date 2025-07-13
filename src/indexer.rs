#![deny(clippy::all)]
#![deny(clippy::pedantic)]

use anyhow::Result;
use flate2::read::GzDecoder;
use lindera::dictionary::{load_dictionary_from_kind, DictionaryKind};
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;
use lindera_tantivy::tokenizer::LinderaTokenizer;
use std::fs::File;
use std::io;
use std::io::{BufReader, Read, Write};
use std::path::Path;
use tantivy::schema::{Schema, TextFieldIndexing, TextOptions, INDEXED, STORED, TEXT};
use tantivy::{Index, TantivyDocument};
use wana_kana::ConvertJapanese;
use xml::reader::XmlEvent;
use xml::EventReader;
use yansi::Paint;

pub fn create_schema() -> Schema {
    let mut builder = Schema::builder();

    let jp_options = TextOptions::default()
        .set_indexing_options(TextFieldIndexing::default().set_tokenizer("ja_JP"))
        .set_stored();

    // ent_seq
    builder.add_i64_field("id", INDEXED | STORED);

    // entry fields
    builder.add_text_field("word", jp_options.clone());
    #[allow(clippy::redundant_clone)]
    builder.add_text_field("reading", jp_options.clone());
    builder.add_text_field("reading_romaji", TEXT | STORED);

    // sense fields
    builder.add_text_field("meaning", TEXT | STORED);
    // part-of-speech
    builder.add_text_field("pos", TEXT | STORED);
    builder.add_text_field("field", TEXT | STORED);

    builder.build()
}

pub fn create_index(schema: &Schema, path: &str, index: &Index) -> Result<()> {
    setup_tokenizer(index)?;
    let mut index_writer = index.writer(50_000_000)?;
    index_writer.delete_all_documents()?;

    let mut parser = create_parser(path)?;
    let schema_fields = extract_schema_fields(schema);
    
    let count = parse_xml_and_index(&mut parser, &mut index_writer, &schema_fields)?;
    
    commit_index(index_writer, count)
}

fn setup_tokenizer(index: &Index) -> Result<()> {
    let dictionary = load_dictionary_from_kind(DictionaryKind::IPADIC)?;
    let segmenter = Segmenter::new(
        Mode::Normal,
        dictionary,
        None, // No user dictionary
    );
    let lindera_tokenizer = LinderaTokenizer::from_segmenter(segmenter);
    index.tokenizers().register("ja_JP", lindera_tokenizer);
    Ok(())
}

fn create_parser(path: &str) -> Result<EventReader<GzDecoder<BufReader<File>>>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let gz = GzDecoder::new(reader);
    Ok(EventReader::new(gz))
}

struct SchemaFields {
    id: tantivy::schema::Field,
    word: tantivy::schema::Field,
    reading: tantivy::schema::Field,
    reading_romaji: tantivy::schema::Field,
    meaning: tantivy::schema::Field,
    pos: tantivy::schema::Field,
    field: tantivy::schema::Field,
}

struct ParseContext {
    glosses: Vec<String>,
    poses: Vec<String>,
    fields: Vec<String>,
    current_entry: Option<TantivyDocument>,
    count: i32,
}

fn extract_schema_fields(schema: &Schema) -> SchemaFields {
    SchemaFields {
        id: schema.get_field("id").unwrap(),
        word: schema.get_field("word").unwrap(),
        reading: schema.get_field("reading").unwrap(),
        reading_romaji: schema.get_field("reading_romaji").unwrap(),
        meaning: schema.get_field("meaning").unwrap(),
        pos: schema.get_field("pos").unwrap(),
        field: schema.get_field("field").unwrap(),
    }
}

fn parse_xml_and_index(
    parser: &mut EventReader<GzDecoder<BufReader<File>>>,
    index_writer: &mut tantivy::IndexWriter,
    schema_fields: &SchemaFields,
) -> Result<i32> {
    let mut context = ParseContext {
        glosses: Vec::new(),
        poses: Vec::new(),
        fields: Vec::new(),
        current_entry: Some(tantivy::doc!()),
        count: 0,
    };

    while let Ok(e) = parser.next() {
        match e {
            XmlEvent::StartElement { name, .. } => {
                handle_start_element(
                    &name.local_name,
                    parser,
                    &mut context,
                    schema_fields,
                );
            }
            XmlEvent::EndElement { name } => {
                if handle_end_element(
                    &name.local_name,
                    &mut context,
                    index_writer,
                    schema_fields,
                )? {
                    // Clear collections for next sense
                    context.glosses.clear();
                    context.poses.clear();
                    context.fields.clear();
                }
            }
            XmlEvent::EndDocument => break,
            _ => {}
        }
    }

    Ok(context.count)
}

fn handle_start_element(
    element_name: &str,
    parser: &mut EventReader<GzDecoder<BufReader<File>>>,
    context: &mut ParseContext,
    schema_fields: &SchemaFields,
) {
    match element_name {
        "entry" => {
            context.current_entry = Some(tantivy::doc!());
        }
        "sense" => {
            context.glosses.clear();
            context.poses.clear();
            context.fields.clear();
        }
        "ent_seq" => {
            let entry_id = extract_next_string(parser);
            context.current_entry
                .as_mut()
                .unwrap()
                .add_i64(schema_fields.id, entry_id.parse::<i64>().unwrap());
        }
        "keb" => {
            let keb = extract_next_string(parser);
            context.current_entry.as_mut().unwrap().add_text(schema_fields.word, keb);
        }
        "reb" => {
            let reb = extract_next_string(parser);
            context.current_entry
                .as_mut()
                .unwrap()
                .add_text(schema_fields.reading, reb.clone());
            context.current_entry
                .as_mut()
                .unwrap()
                .add_text(schema_fields.reading_romaji, reb.to_romaji());
        }
        "gloss" => {
            let gloss = extract_next_string(parser);
            context.glosses.push(gloss);
        }
        "pos" => {
            let pos_value = extract_next_string(parser);
            context.poses.push(pos_value);
        }
        "field" => {
            let field_value = extract_next_string(parser);
            context.fields.push(field_value);
        }
        _ => {}
    }
}

fn handle_end_element(
    element_name: &str,
    context: &mut ParseContext,
    index_writer: &mut tantivy::IndexWriter,
    schema_fields: &SchemaFields,
) -> Result<bool> {
    match element_name {
        "entry" => {
            let current_doc = context.current_entry.take().unwrap();
            index_writer.add_document(current_doc)?;
            context.count += 1;

            if context.count % 1000 == 0 {
                println!("{} entries read...", Paint::default(context.count).bold());
            }
            Ok(false)
        }
        "sense" => {
            if let Some(entry) = context.current_entry.as_mut() {
                entry.add_text(schema_fields.meaning, context.glosses.join("; "));
                entry.add_text(schema_fields.pos, context.poses.join("; "));
                entry.add_text(schema_fields.field, context.fields.join("; "));
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn commit_index(mut index_writer: tantivy::IndexWriter, count: i32) -> Result<()> {
    print!(
        "{} entries read... ",
        Paint::default(count.to_string()).bold()
    );
    io::stdout().flush().unwrap();
    index_writer.commit()?;
    println!("and committed.");
    Ok(())
}

fn extract_next_string<R: Read>(parser: &mut EventReader<R>) -> String {
    let mut buf = String::new();
    loop {
        match parser.next().unwrap() {
            XmlEvent::Characters(s) => {
                buf.push_str(&s);
            }
            XmlEvent::EndElement { name } => {
                if name.local_name == "keb"
                    || name.local_name == "reb"
                    || name.local_name == "gloss"
                    || name.local_name == "pos"
                    || name.local_name == "field"
                    || name.local_name == "ent_seq"
                {
                    break;
                }
            }
            _ => {}
        }
    }
    buf
}

pub fn fetch_jmdict<P: AsRef<Path>>(url: &str, out_file: P) -> Result<()> {
    println!("Downloading JMdict from {url}...");
    let mut resp = reqwest::blocking::get(url)?;
    let mut out = File::create(out_file)?;
    io::copy(&mut resp, &mut out)?;
    println!("Download complete.");
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_extract_next_string() {
        let mut parser = EventReader::from_str(
            r"
            <entry>
                <ent_seq>1</ent_seq>
                <k_ele>
                    <keb>日本</keb>
                </k_ele>
                <r_ele>
                    <reb>にほん</reb>
                </r_ele>
                <sense>
                    <gloss>Japan</gloss>
                    <gloss>Japanese</gloss>
                    <pos>noun</pos>
                    <pos>proper noun</pos>
                    <field>place</field>
                    <field>country</field>
                </sense>
            </entry>
        ",
        );

        assert_eq!(extract_next_string(&mut parser), "1");
        assert_eq!(extract_next_string(&mut parser), "日本");
        assert_eq!(extract_next_string(&mut parser), "にほん");
        assert_eq!(extract_next_string(&mut parser), "Japan");
        assert_eq!(extract_next_string(&mut parser), "Japanese");
        assert_eq!(extract_next_string(&mut parser), "noun");
        assert_eq!(extract_next_string(&mut parser), "proper noun");
        assert_eq!(extract_next_string(&mut parser), "place");
        assert_eq!(extract_next_string(&mut parser), "country");
    }

    #[test]
    fn test_create_index() {
        // download jmdict_e if not present
        let jmdict_path = Path::new("testdata/JMdict_e_test.gz");
        let index_path = tempfile::tempdir().unwrap();
        let schema = create_schema();
        let index = Index::create_in_dir(index_path.path(), schema.clone()).unwrap();
        create_index(&schema, jmdict_path.to_str().unwrap(), &index).unwrap();
    }
}
