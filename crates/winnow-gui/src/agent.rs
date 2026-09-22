//! Help for AI agents that prepare folders for review: `--agent-help` prints
//! the guide, `--install-skill` installs it as a Claude Code skill so agents
//! find it on their own. Both come from the same embedded `agent_guide.md`.

use std::io;
use std::path::PathBuf;

const GUIDE: &str = include_str!("agent_guide.md");

const SKILL_DESCRIPTION: &str = "How to prepare image folders for review in winnow (the \
    keyboard-driven image culling / sorting tool): writing a metadata.csv so images can be \
    sorted and ranked by criteria like model confidence or damage, and setting up buckets \
    (classes) via _class folders or .winnow.toml. Use when selecting images or training data \
    for the user to go through in winnow, or when asked to write winnow metadata.";

pub fn guide() -> String {
    format!("<!-- winnow {} -->\n{GUIDE}", env!("CARGO_PKG_VERSION"))
}

/// Write `~/.claude/skills/winnow/SKILL.md`; returns its path.
pub fn install_skill() -> io::Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| io::Error::new(io::ErrorKind::NotFound, "HOME not set"))?;
    let dir = PathBuf::from(home).join(".claude/skills/winnow");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("SKILL.md");
    let body = format!(
        "---\nname: winnow\ndescription: {SKILL_DESCRIPTION}\n---\n\n{}\n\
         _Installed by `winnow --install-skill`; `winnow --agent-help` prints the guide for the \
         installed winnow version. Re-run `winnow --install-skill` after upgrading._\n",
        guide()
    );
    std::fs::write(&path, body)?;
    Ok(path)
}
