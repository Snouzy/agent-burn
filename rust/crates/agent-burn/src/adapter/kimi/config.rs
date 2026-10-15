use std::{fs, path::Path};

const CONFIG_FILE_NAME: &str = "config.toml";
const KIMI_CODE_ALIAS_PREFIX: &str = "kimi-code/";

/// The few string settings agent-burn reads from Kimi Code's `config.toml`:
/// model aliases and the managed provider's base URL / OAuth slot. A minimal
/// line reader covers what Kimi Code writes (`[a."b".c]` headers, dotted keys,
/// and `key = "string"` values) without pulling in a TOML dependency.
#[derive(Debug, Default)]
pub(super) struct KimiConfig {
    entries: Vec<(Vec<String>, String)>,
}

impl KimiConfig {
    pub(super) fn load(root: &Path) -> Self {
        fs::read_to_string(root.join(CONFIG_FILE_NAME))
            .map(|content| Self::parse(&content))
            .unwrap_or_default()
    }

    pub(super) fn parse(content: &str) -> Self {
        let mut entries = Vec::new();
        let mut table = Vec::new();
        for line in content.lines() {
            let line = strip_comment(line).trim();
            if line.starts_with("[[") {
                // Arrays of tables hold nothing agent-burn reads.
                table = vec!["[[".to_string()];
                continue;
            }
            if let Some(header) = line
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
            {
                table = split_dotted(header).unwrap_or_default();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (Some(key), Some(value)) = (split_dotted(key), parse_string(value.trim())) else {
                continue;
            };
            let mut path = table.clone();
            path.extend(key);
            entries.push((path, value));
        }
        Self { entries }
    }

    /// Last string value at `path` (a table path followed by the key).
    pub(super) fn get(&self, path: &[&str]) -> Option<&str> {
        self.entries
            .iter()
            .rev()
            .find(|(entry, _)| entry.iter().map(String::as_str).eq(path.iter().copied()))
            .map(|(_, value)| value.as_str())
    }

    /// Resolve a wire `model` alias (e.g. `kimi-code/kimi-for-coding`) to the
    /// upstream model id via `[models."<alias>"].model`, falling back to
    /// stripping Kimi Code's managed `kimi-code/` alias prefix.
    pub(super) fn model_for_alias(&self, alias: &str) -> String {
        self.get(&["models", alias, "model"])
            .filter(|model| !model.is_empty())
            .or_else(|| alias.strip_prefix(KIMI_CODE_ALIAS_PREFIX))
            .unwrap_or(alias)
            .to_string()
    }
}

fn strip_comment(line: &str) -> &str {
    let mut quote = None;
    for (index, ch) in line.char_indices() {
        match (quote, ch) {
            (None, '#') => return &line[..index],
            (None, '"' | '\'') => quote = Some(ch),
            (Some(open), _) if ch == open => quote = None,
            _ => {}
        }
    }
    line
}

/// Split a dotted key or table header into segments, unquoting quoted ones.
fn split_dotted(raw: &str) -> Option<Vec<String>> {
    let mut segments = Vec::new();
    let mut rest = raw.trim();
    loop {
        let (segment, after) = if let Some(quoted) = rest.strip_prefix('"') {
            let end = quoted.find('"')?;
            (quoted[..end].to_string(), &quoted[end + 1..])
        } else if let Some(quoted) = rest.strip_prefix('\'') {
            let end = quoted.find('\'')?;
            (quoted[..end].to_string(), &quoted[end + 1..])
        } else {
            let end = rest.find('.').unwrap_or(rest.len());
            let bare = rest[..end].trim();
            if bare.is_empty() {
                return None;
            }
            (bare.to_string(), &rest[end..])
        };
        segments.push(segment);
        let after = after.trim_start();
        if after.is_empty() {
            return Some(segments);
        }
        rest = after.strip_prefix('.')?.trim_start();
    }
}

fn parse_string(raw: &str) -> Option<String> {
    if let Some(literal) = raw.strip_prefix('\'') {
        return literal.strip_suffix('\'').map(str::to_string);
    }
    let basic = raw.strip_prefix('"')?.strip_suffix('"')?;
    let mut value = String::with_capacity(basic.len());
    let mut chars = basic.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            value.push(ch);
            continue;
        }
        match chars.next()? {
            'n' => value.push('\n'),
            't' => value.push('\t'),
            other => value.push(other),
        }
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"
default_model = "kimi-code/kimi-for-coding" # trailing comment

[providers."managed:kimi-code"]
type = "kimi"
base_url = "https://api.kimi.ai/coding/v1"

[providers."managed:kimi-code".oauth]
storage = "file"
key = "oauth/kimi-code-env-0123456789abcdef"

[models."kimi-code/kimi-for-coding"]
provider = "managed:kimi-code"
model = "kimi-for-coding"
max_context_size = 262144

[models.fast]
model = 'kimi-k3'
"#;

    #[test]
    fn reads_quoted_table_paths_and_string_values() {
        let config = KimiConfig::parse(CONFIG);

        assert_eq!(
            config.get(&["providers", "managed:kimi-code", "base_url"]),
            Some("https://api.kimi.ai/coding/v1")
        );
        assert_eq!(
            config.get(&["providers", "managed:kimi-code", "oauth", "key"]),
            Some("oauth/kimi-code-env-0123456789abcdef")
        );
        assert_eq!(
            config.get(&["default_model"]),
            Some("kimi-code/kimi-for-coding")
        );
        assert_eq!(
            config.get(&["models", "kimi-code/kimi-for-coding", "max_context_size"]),
            None
        );
    }

    #[test]
    fn reads_dotted_keys_inside_a_table() {
        let config = KimiConfig::parse(
            "[providers.\"managed:kimi-code\"]\noauth.key = \"oauth/kimi-code\"\n",
        );

        assert_eq!(
            config.get(&["providers", "managed:kimi-code", "oauth", "key"]),
            Some("oauth/kimi-code")
        );
    }

    #[test]
    fn resolves_model_aliases_from_config_then_strips_the_managed_prefix() {
        let config = KimiConfig::parse(CONFIG);

        assert_eq!(
            config.model_for_alias("kimi-code/kimi-for-coding"),
            "kimi-for-coding"
        );
        assert_eq!(config.model_for_alias("fast"), "kimi-k3");
        assert_eq!(config.model_for_alias("kimi-code/k3"), "k3");
        assert_eq!(config.model_for_alias("openai/gpt-5.5"), "openai/gpt-5.5");
    }
}
