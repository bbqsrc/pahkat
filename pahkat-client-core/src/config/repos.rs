use hashbrown::HashMap;
use indexmap::IndexMap;
use std::fmt;
use std::fs::{self, create_dir_all, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use url::Url;

use serde::de::{self, Deserializer, Visitor};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use crate::defaults;
use crate::{LoadedRepository, PackageKey};

use once_cell::sync::{Lazy, OnceCell};
use thiserror::Error;

use super::path::ConfigPath;
use super::FileError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoRecord {
    pub channel: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ReposFile(IndexMap<Url, RepoRecord>);

impl ReposFile {
    fn load<P: AsRef<Path>>(path: P) -> Result<ReposFile, FileError> {
        let file = std::fs::read_to_string(path).map_err(FileError::Read)?;
        let file = toml::from_str(&file)?;
        Ok(file)
    }

    fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), FileError> {
        let mut file = File::create(path).map_err(FileError::Write)?;
        let b = toml::to_vec(&self)?;
        file.write_all(&b).map_err(FileError::Write)?;
        Ok(())
    }

    fn create<P: AsRef<Path>>(path: P) -> Result<ReposFile, FileError> {
        let file = Self::default();
        file.save(path)?;
        Ok(file)
    }
}

#[derive(Debug, Clone)]
pub struct Repos {
    path: PathBuf,
    data: ReposFile,
    is_read_only: bool,
}

impl std::ops::Deref for Repos {
    type Target = IndexMap<Url, RepoRecord>;

    fn deref(&self) -> &Self::Target {
        &self.data.0
    }
}

impl Repos {
    pub fn create<P: AsRef<Path>>(path: P) -> Result<Repos, FileError> {
        let data = ReposFile::create(path.as_ref())?;

        Ok(Repos {
            path: path.as_ref().to_path_buf(),
            data,
            is_read_only: false,
        })
    }

    pub fn load<P: AsRef<Path>>(path: P, is_read_only: bool) -> Result<Repos, FileError> {
        let data = ReposFile::load(path.as_ref())?;

        Ok(Repos {
            path: path.as_ref().to_path_buf(),
            data,
            is_read_only,
        })
    }

    fn reload(&mut self) -> Result<(), FileError> {
        self.data = ReposFile::load(&self.path)?;
        Ok(())
    }

    fn save(&self) -> Result<(), FileError> {
        if self.is_read_only {
            return Err(FileError::ReadOnly);
        }
        self.data.save(&self.path)
    }
}