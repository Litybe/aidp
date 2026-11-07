use std::fs;
use std::path::PathBuf;

const PROMPT_CALL: &str = "safeAgent(\"";
const LABEL_KEY: &str = "label: \"";
const UNLABELED_PROMPT: &str = "stage";
const EXAMPLES_DIR: &str = "tests/snapshots/examples";
const UPDATE_ENV: &str = "INSTA_UPDATE";
const UPDATE_ALWAYS: &str = "always";
const BACKSLASH: char = '\\';

/// Swaps every stage prompt for a `{{example:…}}` marker and checks it against its `.txt` file.
/// `INSTA_UPDATE=always` rewrites the `.txt` files instead, like it does for the snapshots.
pub fn externalize_prompts(script: &str, template: &str) -> String {
    let mut output = String::with_capacity(script.len());
    let mut used_names: Vec<String> = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = script[cursor..].find(PROMPT_CALL) {
        let literal_start = cursor + offset + PROMPT_CALL.len();
        let literal_end = find_literal_end(script, literal_start);
        let example_name = unique_name(&label_after(script, literal_end), &mut used_names);
        let relative_path = format!("{template}/{example_name}.txt");
        check_or_write_example(
            &relative_path,
            &decode_js_literal(&script[literal_start..literal_end]),
        );
        output.push_str(&script[cursor..literal_start]);
        output.push_str(&format!("{{{{example:{relative_path}}}}}"));
        cursor = literal_end;
    }
    output.push_str(&script[cursor..]);
    output
}

fn find_literal_end(script: &str, start: usize) -> usize {
    let bytes = script.as_bytes();
    let mut index = start;
    while bytes[index] != b'"' {
        index += if bytes[index] == BACKSLASH as u8 {
            2
        } else {
            1
        };
    }
    index
}

fn label_after(script: &str, from: usize) -> String {
    let line_end = script[from..]
        .find('\n')
        .map_or(script.len(), |offset| from + offset);
    let window = &script[from..line_end];
    let Some(offset) = window.find(LABEL_KEY) else {
        return UNLABELED_PROMPT.to_string();
    };
    let label_start = offset + LABEL_KEY.len();
    let label_len = window[label_start..].find('"').unwrap_or(0);
    window[label_start..label_start + label_len].to_string()
}

fn unique_name(label: &str, used_names: &mut Vec<String>) -> String {
    let base: String = label
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    let occurrences = used_names.iter().filter(|name| **name == base).count();
    used_names.push(base.clone());
    if occurrences == 0 {
        base
    } else {
        format!("{base}-{}", occurrences + 1)
    }
}

fn decode_js_literal(literal: &str) -> String {
    let mut decoded = String::with_capacity(literal.len());
    let mut characters = literal.chars();
    while let Some(character) = characters.next() {
        if character != BACKSLASH {
            decoded.push(character);
            continue;
        }
        match characters.next() {
            Some('n') => decoded.push('\n'),
            Some('t') => decoded.push('\t'),
            Some('r') => decoded.push('\r'),
            Some('u') => {
                let hex: String = characters.by_ref().take(4).collect();
                let code_point =
                    u32::from_str_radix(&hex, 16).expect("valid unicode escape in emitted prompt");
                decoded
                    .push(char::from_u32(code_point).expect("valid code point in emitted prompt"));
            }
            Some(other) => decoded.push(other),
            None => decoded.push(BACKSLASH),
        }
    }
    decoded
}

fn check_or_write_example(relative_path: &str, prompt: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(EXAMPLES_DIR)
        .join(relative_path);
    if std::env::var(UPDATE_ENV).as_deref() == Ok(UPDATE_ALWAYS) {
        fs::create_dir_all(path.parent().expect("example path has a parent"))
            .expect("create examples dir");
        fs::write(&path, prompt).expect("write prompt example");
        return;
    }
    let expected = fs::read_to_string(&path)
        .unwrap_or_else(|error| {
            panic!(
                "missing prompt example {}: {error} (regenerate with INSTA_UPDATE=always)",
                path.display()
            )
        })
        .replace("\r\n", "\n");
    assert_eq!(
        expected,
        prompt,
        "stage prompt differs from {}",
        path.display()
    );
}
