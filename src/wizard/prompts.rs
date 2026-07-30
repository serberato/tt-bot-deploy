//! CLI input prompting utilities for the interactive setup wizard.

use std::io::{self, Write};

use crate::wizard::validation::parse_yes_no_input;

/// Prompt the user for string input with a display default and optional required flag.
pub(crate) fn ask(prompt: &str, default: &str, required: bool) -> Option<String> {
    loop {
        if default.is_empty() {
            print!("  {prompt}: ");
        } else {
            print!("  {prompt} [{default}]: ");
        }
        io::stdout().flush().ok();

        let mut input = String::new();
        match io::stdin().read_line(&mut input) {
            Ok(0) | Err(_) => {
                println!("\nSetup cancelled.");
                return None;
            }
            _ => {}
        }

        let input = input.trim().to_string();
        if input.is_empty() && !default.is_empty() {
            return Some(default.to_string());
        }
        if input.is_empty() && required {
            println!("    This field is required.");
            continue;
        }
        return Some(input);
    }
}

/// Prompt the user for an integer input, retrying until valid.
pub(crate) fn ask_int(prompt: &str, default: i32) -> Option<i32> {
    loop {
        let raw = ask(prompt, &default.to_string(), true)?;
        match raw.parse::<i32>() {
            Ok(v) => return Some(v),
            Err(_) => println!("    Invalid input. Expected a number."),
        }
    }
}

/// Prompt for a boolean yes/no choice.
pub(crate) fn ask_yes_no(prompt: &str, default: bool) -> Option<bool> {
    let default_str = if default { "Y/n" } else { "y/N" };
    let prompt_text = format!("{prompt} ({default_str})");
    let ans = ask(&prompt_text, if default { "y" } else { "n" }, false)?;
    Some(parse_yes_no_input(&ans, default))
}
