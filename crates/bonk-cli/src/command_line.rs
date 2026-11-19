use clap::Parser;

use crate::*;

/// Checks a command line is accepted by the CLI without running it, and returns its words: every equivalent
/// command the window shows must be valid.
pub fn check_command_line(command: &str) -> Result<Vec<String>, String> {
    let words = split_words(command)?;
    Cli::try_parse_from(&words).map_err(|erreur| erreur.to_string())?;
    Ok(words)
}

/// Checks the arguments an interface passes to `bonk` are accepted by the CLI, without running it.
pub fn check_args(args: &[String]) -> Result<(), String> {
    Cli::try_parse_from(std::iter::once("bonk").chain(args.iter().map(String::as_str)))
        .map(drop)
        .map_err(|erreur| erreur.to_string())
}

/// POSIX-shell-like splitting: spaces, single quotes, double quotes. Variables and substitutions are not
/// interpreted (the window produces none).
pub(crate) fn split_words(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => current.push(c),
            // Outside quotes a backslash makes the next character literal (`'\''` in the text of an id
            // containing an apostrophe).
            (None, '\\') => {
                current.push(chars.next().ok_or("trailing backslash")?);
                in_word = true;
            }
            (None, '\'' | '"') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            (None, c) => {
                current.push(c);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        return Err("unclosed quote".to_owned());
    }
    if in_word {
        words.push(current);
    }
    Ok(words)
}
