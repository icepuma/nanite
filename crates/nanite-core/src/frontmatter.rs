use anyhow::{Result, anyhow};
use serde::de::DeserializeOwned;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontmatterDocument<T> {
    pub metadata: T,
    pub body: String,
}

pub fn parse_frontmatter<T>(source: &str) -> Result<FrontmatterDocument<T>>
where
    T: DeserializeOwned,
{
    let stripped = source
        .strip_prefix("---\n")
        .ok_or_else(|| anyhow!("missing frontmatter start"))?;
    let (frontmatter, body) = stripped
        .split_once("\n---\n")
        .ok_or_else(|| anyhow!("missing frontmatter end"))?;

    let metadata = serde_yaml::from_str(frontmatter)?;
    Ok(FrontmatterDocument {
        metadata,
        body: body.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::parse_frontmatter;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct Meta {
        name: String,
    }

    #[test]
    fn parses_yaml_frontmatter_and_body() {
        let document = parse_frontmatter::<Meta>("---\nname: test\n---\nhello\n").unwrap();

        assert_eq!(
            document.metadata,
            Meta {
                name: "test".to_owned()
            }
        );
        assert_eq!(document.body, "hello\n");
    }
}
