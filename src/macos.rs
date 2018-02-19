use pahkat::types::*;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::ffi::OsStr;
use xz2::read::XzDecoder;
use std::fs::{remove_file, read_dir, remove_dir, create_dir_all, File};
use std::cell::RefCell;
use rhai::RegisterFn;
use std::fmt::Display;
use std::str::FromStr;
use std::process::Command;
use std::collections::BTreeMap;
use ::{Repository};

use serde::de::{self, Deserialize, Deserializer};
use plist::serde::{deserialize as deserialize_plist};

use ::*;


fn from_str<'de, T, D>(deserializer: D) -> Result<T, D::Error>
    where T: FromStr,
          T::Err: Display,
          D: Deserializer<'de>
{
    let s = String::deserialize(deserializer)?;
    T::from_str(&s).map_err(de::Error::custom)
}

#[derive(Debug, Deserialize)]
struct BundlePlistInfo {
    #[serde(rename = "CFBundleIdentifier")]
    pub identifier: Option<String>,
    #[serde(rename = "CFBundleName")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "from_str", rename = "CFBundleVersion")]
    pub version: usize,
    #[serde(rename = "CFBundleShortVersionString")]
    pub short_version: Option<String>,
}

#[test]
fn test_bundle_plist() {
    let file = File::open("/Users/Brendan/Library/Keyboard Layouts/so.brendan.keyboards.keyboardlayout.brendan.bundle/Contents/Info.plist").unwrap();
    let plist: BundlePlistInfo = deserialize_plist(file).unwrap();
    println!("{:?}", plist);
}

#[derive(Debug, Clone, Copy)]
pub enum MacOSInstallError {
    NoInstaller,
    WrongInstallerType,
    InvalidFileType,
    PackageNotInCache
}

#[derive(Debug, Clone, Copy)]
pub enum MacOSUninstallError {
    NoInstaller,
    WrongInstallerType
}

pub struct MacOSPackageStore<'a> {
    repo: &'a Repository,
    config: &'a StoreConfig
}

impl<'a> MacOSPackageStore<'a> {
    pub fn new(repo: &'a Repository, config: &'a StoreConfig) -> MacOSPackageStore<'a> {
        MacOSPackageStore { repo: repo, config: config }
    }

    // TODO: review if there is a better place to put this function...
    pub fn download_path(&self, _package: &Package) -> PathBuf {
        return Path::new(&self.config.cache_dir).join(self.repo.hash_id())
    }

    pub fn install(&self, package: &'a Package, target: MacOSInstallTarget) -> Result<PackageStatus, MacOSInstallError> {
        let installer = match package.installer() {
            None => return Err(MacOSInstallError::NoInstaller),
            Some(v) => v
        };

        let installer = match *installer {
            Installer::MacOS(ref v) => v,
            _ => return Err(MacOSInstallError::WrongInstallerType)
        };

        let url = url::Url::parse(&installer.url).unwrap();
        let filename = url.path_segments().unwrap().last().unwrap();
        let pkg_path = self.download_path(&package).join(filename);

        if !pkg_path.exists() {
            return Err(MacOSInstallError::PackageNotInCache)
        }
        
        install_macos_package(&pkg_path, target).unwrap();

        Ok(self.status_impl(installer, package, target).unwrap())
    }

    pub fn uninstall(&self, package: &'a Package, target: MacOSInstallTarget) -> Result<PackageStatus, MacOSUninstallError> {
        let installer = match package.installer() {
            None => return Err(MacOSUninstallError::NoInstaller),
            Some(v) => v
        };

        let installer = match installer {
            &Installer::MacOS(ref v) => v,
            _ => return Err(MacOSUninstallError::WrongInstallerType)
        };

        uninstall_macos_package(&installer.pkg_id, target).unwrap();

        Ok(self.status_impl(installer, package, target).unwrap())
    }

    pub fn status(&self, package: &'a Package, target: MacOSInstallTarget) -> Result<PackageStatus, PackageStatusError> {
        let installer = match package.installer() {
            None => return Err(PackageStatusError::NoInstaller),
            Some(v) => v
        };

        let installer = match installer {
            &Installer::MacOS(ref v) => v,
            _ => return Err(PackageStatusError::WrongInstallerType)
        };

        self.status_impl(installer, package, target)
    }

    fn status_impl(&self, installer: &MacOSInstaller, package: &'a Package, target: MacOSInstallTarget) -> Result<PackageStatus, PackageStatusError> {
        let pkg_info = match get_package_info(&installer.pkg_id, target) {
            Some(v) => v,
            None => return Ok(PackageStatus::NotInstalled)
        };

        let installed_version = match semver::Version::parse(&pkg_info.pkg_version) {
            Err(_) => return Err(PackageStatusError::ParsingVersion),
            Ok(v) => v
        };

        let candidate_version = match semver::Version::parse(&package.version) {
            Err(_) => return Err(PackageStatusError::ParsingVersion),
            Ok(v) => v
        };

        if candidate_version > installed_version {
            Ok(PackageStatus::RequiresUpdate)
        } else {
            Ok(PackageStatus::UpToDate)
        }
    }
}

fn get_installed_packages(target: MacOSInstallTarget) -> Result<Vec<String>, io::Error> {
    use std::io::Cursor;
    use std::env;

    let home_dir = env::home_dir().expect("Always find home directory");
    
    let mut args = vec!["--pkgs-plist"];
    if let MacOSInstallTarget::User = target {
        args.push("--volume");
        args.push(&home_dir.to_str().unwrap());
    }

    let output = Command::new("pkgutil").args(&args).output()?;
    let plist_data = String::from_utf8(output.stdout).expect("plist should always be valid UTF-8");
    let cursor = Cursor::new(plist_data);
    let plist: Vec<String> = deserialize_plist(cursor).expect("plist should always be valid");
    return Ok(plist);
}

#[derive(Debug, Deserialize)]
struct MacOSPackageExportPath {
    pub gid: u64,
    #[serde(rename = "install-time")]
    pub install_time: u64,
    pub mode: u64,
    #[serde(rename = "pkg-version")]
    pub pkg_version: String,
    pub pkgid: String,
    pub uid: u64
}

#[derive(Debug, Deserialize)]
struct MacOSPackageExportPlist {
    #[serde(rename = "install-location")]
    pub install_location: String,
    #[serde(rename = "install-time")]
    pub install_time: u64,
    pub paths: BTreeMap<String, MacOSPackageExportPath>,
    #[serde(rename = "pkg-version")]
    pub pkg_version: String,
    pub pkgid: String,
    #[serde(rename = "receipt-plist-version")]
    pub receipt_plist_version: f64,
    pub volume: String
}

impl MacOSPackageExportPlist {
    fn path(&self) -> PathBuf {
        Path::new(&self.volume).join(&self.install_location)
    }

    fn paths(&self) -> Vec<PathBuf> {
        let base_path = self.path();
        self.paths.keys().map(|p| base_path.join(p)).collect()
    }
}

fn get_package_info(bundle_id: &str, target: MacOSInstallTarget) -> Option<MacOSPackageExportPlist> {
    use std::io::Cursor;
    use std::env;

    let home_dir = env::home_dir().expect("Always find home directory");
    let mut args = vec!["--export-plist", bundle_id];
    if let MacOSInstallTarget::User = target {
        args.push("--volume");
        args.push(&home_dir.to_str().unwrap());
    }

    let res = Command::new("pkgutil").args(&args).output();
    let output = match res {
        Ok(v) => v,
        Err(_) => return None
    };
    if !output.status.success() {
        return None;
    }
    let plist_data = String::from_utf8(output.stdout).expect("plist should always be valid UTF-8");
    let cursor = Cursor::new(plist_data);
    let plist: MacOSPackageExportPlist = deserialize_plist(cursor).expect("plist should always be valid");
    return Some(plist);
}

// #[test]
fn test_get_pkgs() {
    println!("{:?}", get_installed_packages(MacOSInstallTarget::User))
}

// #[test]
fn test_get_pkg_info() {
    let pkg_info = get_package_info("com.oracle.jre", MacOSInstallTarget::System).unwrap();
    // println!("{:?}", &pkg_info);
    println!("{:?}", &pkg_info.paths());
}

#[derive(Debug)]
enum ProcessError {
    Io(io::Error),
    Unknown(String)
}

fn install_macos_package(pkg_path: &Path, target: MacOSInstallTarget) -> Result<(), ProcessError> {
    let target_str = match target {
        MacOSInstallTarget::User => "CurrentUserHomeDirectory",
        MacOSInstallTarget::System => "LocalSystem"
    };

    let args = &[
        "-pkg",
        &pkg_path.to_str().unwrap(),
        "-target",
        target_str
    ];

    let res = Command::new("installer").args(args).output();
    let output = match res {
        Ok(v) => v,
        Err(e) => return Err(ProcessError::Io(e))
    };
    if !output.status.success() {
        return Err(ProcessError::Unknown(String::from_utf8_lossy(&output.stderr).to_string()));
    }
    Ok(())
}

fn uninstall_macos_package(bundle_id: &str, target: MacOSInstallTarget) -> Result<(), ()> {
    use std::fs;
    use std::env;
    
    let package_info = get_package_info(bundle_id, target).unwrap();

    let mut errors = vec![];
    let mut directories = vec![];

    for path in package_info.paths() {
        if path.is_dir() {
            directories.push(path);
            continue;
        }

        println!("Deleting: {:?}", &path);
        fs::remove_file(path).unwrap();
    }

    // Ensure children are deleted first
    directories.sort_unstable_by(|a, b| {
        let a_count = a.to_string_lossy().chars().filter(|x| *x == '/').count();
        let b_count = b.to_string_lossy().chars().filter(|x| *x == '/').count();
        b_count.cmp(&a_count)
    });

    for dir in directories {
        println!("Deleting: {:?}", &dir);
        match fs::remove_dir(dir) {
            Err(e) => errors.push(e),
            Ok(_) => {}
        }
    }

    println!("{:?}", errors);
    
    forget_pkg_id(bundle_id, target).unwrap();

    Ok(())
}

fn forget_pkg_id(bundle_id: &str, target: MacOSInstallTarget) -> Result<(), ()> {
    use std::env;

    let home_dir = env::home_dir().expect("Always find home directory");
    let mut args = vec!["--forget", bundle_id];
    if let MacOSInstallTarget::User = target {
        args.push("--volume");
        args.push(&home_dir.to_str().unwrap());
    }

    let res = Command::new("pkgutil").args(&args).output();
    let output = match res {
        Ok(v) => v,
        Err(e) => {
            println!("{:?}", e);
            return Err(())
        }
    };
    if !output.status.success() {
        println!("{:?}", output.status.code().unwrap());
        return Err(());
    }
    Ok(())
}

// #[test]
// fn end_to_end() {
//     let store = MacOSPackageStore::new(Path::new("/tmp"));
//     install_macos_package(&Path::new("/Users/brendan/git/kbdgen/outputs/sme/North_Sami_Keyboard_1.0.1.unsigned.pkg"), MacOSInstallTarget::User).unwrap();
//     uninstall_macos_package("no.uit.giella.keyboardlayout.sme", MacOSInstallTarget::User).unwrap()
// }
