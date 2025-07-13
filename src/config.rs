use std::{io::Read, path::PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Default, PartialEq, Eq, Clone)]
pub(crate) struct Config {
    #[serde(skip_serializing_if = "Index::is_default")]
    pub(crate) index: Index,
    #[serde(skip_serializing_if = "Jmdict::is_default")]
    pub(crate) jmdict: Jmdict,
}

#[derive(Debug, Deserialize, Serialize, Default, PartialEq, Eq, Clone)]
pub(crate) struct Index {
    pub(crate) path: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Serialize, Default, PartialEq, Eq, Clone)]
pub(crate) struct Jmdict {
    pub(crate) path: Option<PathBuf>,
    pub(crate) url: Option<String>,
}

impl Config {
    pub(crate) fn from_file<P: Into<PathBuf>>(path: P) -> Result<Self> {
        let path = path.into();

        let mut file = std::fs::File::open(path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;

        let config = toml::from_str(&contents)?;

        Ok(config)
    }
    #[cfg(test)]
    pub(crate) fn from_str(s: &str) -> Result<Self> {
        let config = toml::from_str(s)?;

        Ok(config)
    }
    pub(crate) fn to_str(&self) -> Result<String> {
        toml::to_string(self).map_err(std::convert::Into::into)
    }
}

impl Index {
    pub(crate) fn is_default(&self) -> bool {
        matches!(self, Index { path: None })
    }
}

impl Jmdict {
    pub(crate) fn is_default(&self) -> bool {
        matches!(
            self,
            Jmdict {
                path: None,
                url: None,
            }
        )
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_config_from_str() {
        let config = Config::from_str(
            r#"
            [index]
            path = "/tmp/index"

            [jmdict]
            path = "/tmp/jmdict"
            url = "https://ftp.monash.edu/pub/nihongo/JMdict_e.gz"
            "#,
        )
        .unwrap();

        assert_eq!(
            config,
            Config {
                index: Index {
                    path: Some("/tmp/index".into())
                },
                jmdict: Jmdict {
                    path: Some("/tmp/jmdict".into()),
                    url: Some("https://ftp.monash.edu/pub/nihongo/JMdict_e.gz".into())
                }
            }
        );
    }

    #[test]
    fn test_config_to_str() {
        let config = Config {
            index: Index {
                path: Some("/tmp/index".into()),
            },
            jmdict: Jmdict {
                path: Some("/tmp/jmdict".into()),
                url: Some("https://ftp.monash.edu/pub/nihongo/JMdict_e.gz".into()),
            },
        };

        assert_eq!(
            config.to_str().unwrap(),
            r#"[index]
path = "/tmp/index"

[jmdict]
path = "/tmp/jmdict"
url = "https://ftp.monash.edu/pub/nihongo/JMdict_e.gz"
"#
        );
    }
    // verify omitted fields are not serialized
    #[test]
    fn test_config_to_str_omit() {
        let config = Config {
            index: Index {
                path: Some("/tmp/index".into()),
            },
            jmdict: Jmdict {
                path: None,
                url: None,
            },
        };

        assert_eq!(
            config.to_str().unwrap(),
            r#"[index]
path = "/tmp/index"
"#
        );
    }
}
