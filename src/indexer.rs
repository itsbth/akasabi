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
use tantivy::Index;
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
    // Register the Japanese tokenizer with IPADIC dictionary
    let dictionary = load_dictionary_from_kind(DictionaryKind::IPADIC)?;
    let segmenter = Segmenter::new(
        Mode::Normal,
        dictionary,
        None, // No user dictionary
    );
    let lindera_tokenizer = LinderaTokenizer::from_segmenter(segmenter);
    index.tokenizers().register("ja_JP", lindera_tokenizer);
    
    let mut index_writer = index.writer(50_000_000)?;

    // Start with a clean slate
    index_writer.delete_all_documents()?;

    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut gz = GzDecoder::new(reader);
    let mut parser = EventReader::new(&mut gz);

    // common fields
    let id = schema.get_field("id").unwrap();

    // entry fields
    let word = schema.get_field("word").unwrap();
    let reading = schema.get_field("reading").unwrap();
    let reading_romaji = schema.get_field("reading_romaji").unwrap();

    // sense fields
    let meaning = schema.get_field("meaning").unwrap();
    let pos = schema.get_field("pos").unwrap();
    let field = schema.get_field("field").unwrap();

    let mut glosses = Vec::new();
    // poss?
    let mut poses = Vec::new();
    // Can this have >1 value?
    let mut fields = Vec::new();

    let mut current_entry = Some(tantivy::doc!());

    let mut count = 0;

    while let Ok(e) = parser.next() {
        match e {
            XmlEvent::StartElement { name, .. } => match name.local_name.as_str() {
                "entry" => {
                    current_entry = Some(tantivy::doc!());
                }
                "sense" => {
                    glosses.clear();
                    poses.clear();
                    fields.clear();
                }
                "ent_seq" => {
                    let entry_id = extract_next_string(&mut parser);
                    current_entry
                        .as_mut()
                        .unwrap()
                        .add_i64(id, entry_id.parse::<i64>().unwrap());
                }
                "keb" => {
                    let keb = extract_next_string(&mut parser);
                    current_entry.as_mut().unwrap().add_text(word, keb);
                }
                "reb" => {
                    let reb = extract_next_string(&mut parser);
                    current_entry
                        .as_mut()
                        .unwrap()
                        .add_text(reading, reb.clone());
                    current_entry
                        .as_mut()
                        .unwrap()
                        .add_text(reading_romaji, reb.to_romaji());
                }
                "gloss" => {
                    let gloss = extract_next_string(&mut parser);
                    glosses.push(gloss);
                }
                "pos" => {
                    let pos_value = extract_next_string(&mut parser);
                    poses.push(pos_value);
                }
                "field" => {
                    let field_value = extract_next_string(&mut parser);
                    fields.push(field_value);
                }
                _ => {}
            },
            XmlEvent::EndElement { name } => {
                if name.local_name == "entry" {
                    let current_doc = current_entry.take().unwrap();
                    index_writer.add_document(current_doc)?;

                    count += 1;

                    if count % 1000 == 0 {
                        println!("{} entries read...", Paint::default(count).bold());
                    }
                } else if name.local_name == "sense" {
                    if let Some(entry) = current_entry.as_mut() {
                        entry.add_text(meaning, glosses.join("; "));
                        entry.add_text(pos, poses.join("; "));
                        entry.add_text(field, fields.join("; "));
                    }
                }
            }
            XmlEvent::EndDocument => {
                // NB: Parser will repeatedly return EndDocument, so we need to break out of the loop
                break;
            }
            _ => {}
        }
    }

    print!(
        "{} entries read... ",
        Paint::default(count.to_string()).bold()
    );
    // Flush stdout so that the progress indicator is displayed
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

fn fetch_jmdict<P: AsRef<Path>>(out_file: P) -> Result<()> {
    let url = "https://ftp.monash.edu/pub/nihongo/JMdict_e.gz";
    let mut resp = reqwest::blocking::get(url)?;
    let mut out = File::create(out_file)?;
    io::copy(&mut resp, &mut out)?;
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
