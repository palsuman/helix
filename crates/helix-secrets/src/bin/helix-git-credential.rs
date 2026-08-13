//! Git credential helper binary invoked by Git (REQ-SEC-002.8).
//!
//! Configure with:
//! `git config --global credential.helper '!helix-git-credential'`

use std::io::{self, Read};

fn main() {
    if let Err(error) = run() {
        eprintln!("helix-git-credential: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let action = std::env::args()
        .nth(1)
        .ok_or("missing git-credential action (expected get, store, or erase)")?;
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let output = helix_secrets::handle_git_credential(&action, &input)?;
    if !output.is_empty() {
        print!("{output}");
    }
    Ok(())
}
