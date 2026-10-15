//! Cheap byte scans for paths that tool calls touch. Tool inputs can be large
//! and nested as JSON inside JSON (or inside JavaScript for Codex code mode),
//! so values are read straight from the raw log line instead of decoding it.

use memchr::memmem;

/// Bytes allowed between a key and its value: quotes, escapes, colons, spaces.
const KEY_SEPARATORS: &[u8] = b"\\\"': ";
const MAX_SEPARATOR_LEN: usize = 8;

/// Absolute path values of `key` (`"file_path":"/x"`, `\"workdir\":\"/x\"`,
/// `workdir: \"/x\"`). Relative values are skipped.
pub(crate) fn key_paths<'a>(line: &'a [u8], key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    memmem::find_iter(line, key.as_bytes()).filter_map(move |start| {
        let before = start.checked_sub(1).map(|index| line[index]);
        if before.is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_') {
            return None;
        }
        let rest = &line[start + key.len()..];
        if !matches!(rest.first(), Some(b'"' | b'\\' | b':')) {
            return None;
        }
        let separator = rest
            .iter()
            .take(MAX_SEPARATOR_LEN)
            .take_while(|byte| KEY_SEPARATORS.contains(byte))
            .count();
        let value = &rest[separator..];
        (value.first() == Some(&b'/')).then(|| path_until(value, b"\\\"'"))?
    })
}

/// Absolute targets of `cd` in shell commands, such as `cd /repo && make`.
pub(crate) fn cd_paths(line: &[u8]) -> impl Iterator<Item = &str> {
    memmem::find_iter(line, b"cd ").filter_map(move |start| {
        let separated = match start.checked_sub(1).map(|index| line[index]) {
            None => true,
            // An escaped newline (`\n`) also starts a command.
            Some(b'n') => start >= 2 && line[start - 2] == b'\\',
            Some(byte) => b" \"'(;&|".contains(&byte),
        };
        if !separated {
            return None;
        }
        let rest = &line[start + 3..];
        let quotes = rest
            .iter()
            .take(3)
            .take_while(|byte| matches!(byte, b'\\' | b'"' | b'\''))
            .count();
        let value = &rest[quotes..];
        (value.first() == Some(&b'/')).then(|| path_until(value, b"\\\"' ;&|)"))?
    })
}

/// File paths from `apply_patch` headers (`*** Update File: src/main.rs`).
/// They may be relative to the call's working directory.
pub(crate) fn patch_paths(line: &[u8]) -> impl Iterator<Item = &str> {
    [
        &b"*** Add File: "[..],
        b"*** Update File: ",
        b"*** Delete File: ",
    ]
    .into_iter()
    .flat_map(move |header| {
        memmem::find_iter(line, header)
            .filter_map(move |start| path_until(&line[start + header.len()..], b"\\\""))
    })
}

fn path_until<'a>(value: &'a [u8], terminators: &[u8]) -> Option<&'a str> {
    let end = value
        .iter()
        .position(|byte| terminators.contains(byte) || *byte == b'\n')
        .unwrap_or(value.len());
    let path = std::str::from_utf8(&value[..end]).ok()?.trim_end();
    (!path.is_empty()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect<'a>(paths: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
        paths.collect()
    }

    #[test]
    fn reads_absolute_json_key_values() {
        let line = br#"{"input":{"file_path":"/repo/src/main.rs","path":"src","notebook_path":"/repo/a.ipynb"}}"#;

        assert_eq!(
            collect(key_paths(line, "file_path")),
            vec!["/repo/src/main.rs"]
        );
        assert_eq!(collect(key_paths(line, "path")), Vec::<&str>::new());
        assert_eq!(
            collect(key_paths(line, "notebook_path")),
            vec!["/repo/a.ipynb"]
        );
    }

    #[test]
    fn reads_nested_and_code_mode_workdir_values() {
        let line = br#"{"arguments":"{\"cmd\":\"ls\",\"workdir\":\"/repo/api\"}","input":"tools.exec_command({cmd:\"ls\", workdir: \"/repo/web\"})"}"#;

        assert_eq!(
            collect(key_paths(line, "workdir")),
            vec!["/repo/api", "/repo/web"]
        );
    }

    #[test]
    fn keeps_spaces_inside_paths() {
        let line = br#"{"file_path":"/Users/me/Mobile Documents/notes.md"}"#;

        assert_eq!(
            collect(key_paths(line, "file_path")),
            vec!["/Users/me/Mobile Documents/notes.md"]
        );
    }

    #[test]
    fn reads_cd_targets_from_shell_commands() {
        let line =
            br#"{"command":"cd /repo/api && cargo test; (cd \"/repo/web\" && ls)\nabcd /nope"}"#;

        assert_eq!(collect(cd_paths(line)), vec!["/repo/api", "/repo/web"]);
    }

    #[test]
    fn reads_apply_patch_headers() {
        let line = br#"{"input":"*** Begin Patch\\n*** Update File: /repo/a.rs\\n@@\\n*** Add File: src/b.rs\\n+x\\n*** Delete File: /repo/c.rs\\n*** End Patch"}"#;

        assert_eq!(
            collect(patch_paths(line)),
            vec!["src/b.rs", "/repo/a.rs", "/repo/c.rs"]
        );
    }
}
