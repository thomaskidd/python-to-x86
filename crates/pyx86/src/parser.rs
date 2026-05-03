use std::path::Path;

use anyhow::{anyhow, Result};
use rustpython_parser::{ast, Parse};

pub type Module = ast::ModModule;

pub fn parse(source: &str, source_path: &Path) -> Result<Module> {
    let path_str = source_path.to_string_lossy();
    ast::ModModule::parse(source, &path_str).map_err(|e| {
        let (line, col) = byte_offset_to_line_col(source, e.offset.to_usize());
        anyhow!("parse: {} \n --> {}:{}:{}", e.error, path_str, line, col)
    })
}

fn byte_offset_to_line_col(source: &str, offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for (i, ch) in source.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parses_trivial_program() {
        let src = "def main() -> int:\n    return 42\n";
        let m = parse(src, &PathBuf::from("test.py")).expect("should parse");
        assert_eq!(m.body.len(), 1);
    }

    #[test]
    fn reports_syntax_error_with_location() {
        let src = "def main() -> int\n    return 42\n"; // missing colon
        let err = parse(src, &PathBuf::from("test.py")).unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("test.py"));
    }
}
