use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf, absolute},
};

use thiserror::Error;

const SKILL_NAME: &str = "using-forkstr";
const FILES: &[(&str, &[u8])] = &[
    (
        "SKILL.md",
        include_bytes!("../skills/using-forkstr/SKILL.md"),
    ),
    (
        "agents/openai.yaml",
        include_bytes!("../skills/using-forkstr/agents/openai.yaml"),
    ),
    (
        "references/configuration.md",
        include_bytes!("../skills/using-forkstr/references/configuration.md"),
    ),
];

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("cannot resolve skill export destination: {0}")]
    Resolve(#[source] io::Error),
    #[error("destination {0} already exists; choose a new directory. Nothing was overwritten")]
    Exists(PathBuf),
    #[error(
        "cannot create {path}: {source}; choose a writable destination with an existing parent directory"
    )]
    Create {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "skill export incomplete at {path}: {source}; review the partial directory and retry with a new destination"
    )]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[derive(Debug)]
pub struct ExportedSkill {
    pub directory: PathBuf,
    pub skill_directory: PathBuf,
    pub files: &'static [(&'static str, &'static [u8])],
}

pub fn export(directory: &Path) -> Result<ExportedSkill, ExportError> {
    let directory = absolute(directory).map_err(ExportError::Resolve)?;
    match fs::create_dir(&directory) {
        Ok(()) => {}
        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
            return Err(ExportError::Exists(directory));
        }
        Err(source) => {
            return Err(ExportError::Create {
                path: directory,
                source,
            });
        }
    }

    let skill_directory = directory.join(SKILL_NAME);
    write_bundle(&skill_directory).map_err(|source| ExportError::Write {
        path: directory.clone(),
        source,
    })?;

    Ok(ExportedSkill {
        directory,
        skill_directory,
        files: FILES,
    })
}

fn write_bundle(directory: &Path) -> io::Result<()> {
    fs::create_dir(directory)?;
    for (relative, contents) in FILES {
        let path = directory.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(contents)?;
    }
    Ok(())
}
