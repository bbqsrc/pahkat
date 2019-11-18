#[cfg(all(target_os = "macos", feature = "macos"))]
pub mod macos;

#[cfg(all(windows, feature = "windows"))]
pub mod windows;

#[cfg(feature = "prefix")]
pub mod prefix;

#[no_mangle]
pub extern "C" fn pahkat_enable_logging() {
    use std::io::Write;

    std::env::set_var("RUST_LOG", "pahkat_client=debug");
    env_logger::builder()
        .format(|buf, record| {
            writeln!(
                buf,
                "{} {} {}:{} > {}",
                record.level(),
                record.target(),
                record.file().unwrap_or("<unknown>"),
                record.line().unwrap_or(0),
                record.args()
            )
        })
        .init();
}

use std::convert::TryFrom;
use std::error::Error;
use std::ffi::{CStr, CString};
use std::sync::{Arc, RwLock};

use cursed::{FromForeign, InputType, ReturnType, ToForeign};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::{PackageKey, StoreConfig};
use crate::repo::RepoRecord;

pub struct JsonMarshaler;

impl InputType for JsonMarshaler {
    type Foreign = <cursed::StringMarshaler as InputType>::Foreign;
}

impl ReturnType for JsonMarshaler {
    type Foreign = *const libc::c_char;

    fn foreign_default() -> Self::Foreign {
        std::ptr::null()
    }
}

impl<T> ToForeign<T, *const libc::c_char> for JsonMarshaler
where
    T: Serialize,
{
    type Error = Box<dyn Error>;

    fn to_foreign(input: T) -> Result<*const libc::c_char, Self::Error> {
        let vec = serde_json::to_vec(&input)?;
        let c_str = CString::new(vec)?;
        log::debug!("JSON MARSHAL: {:?}", &c_str);
        Ok(c_str.into_raw())
    }

    // fn drop_foreign(ptr: *const c_char) {
    //     unsafe { CString::from_raw(ptr as *mut _) };
    // }
}

impl<T> FromForeign<*const libc::c_char, T> for JsonMarshaler
where
    T: DeserializeOwned,
{
    type Error = Box<dyn Error>;

    fn from_foreign(ptr: *const libc::c_char) -> Result<T, Self::Error> {
        let s = unsafe { CStr::from_ptr(ptr) }.to_string_lossy();
        log::debug!("JSON: {}", s);
        serde_json::from_str(&s).map_err(|e| Box::new(e) as _)
    }

    // fn drop_local(ptr: *const c_char) {
    //     unsafe { CString::from_raw(ptr as *mut _) };
    // }
}

pub struct PackageKeyMarshaler;

impl InputType for PackageKeyMarshaler {
    type Foreign = <cursed::StringMarshaler as InputType>::Foreign;
}

impl ReturnType for PackageKeyMarshaler {
    type Foreign = <cursed::StringMarshaler as ReturnType>::Foreign;

    fn foreign_default() -> Self::Foreign {
        std::ptr::null()
    }
}

impl<'a> ToForeign<&'a PackageKey, *const libc::c_char> for PackageKeyMarshaler {
    type Error = Box<dyn Error>;

    fn to_foreign(key: &'a PackageKey) -> Result<*const libc::c_char, Self::Error> {
        cursed::StringMarshaler::to_foreign(key.to_string())
    }
}

impl FromForeign<*const libc::c_char, PackageKey> for PackageKeyMarshaler {
    type Error = Box<dyn Error>;

    fn from_foreign(string: *const libc::c_char) -> Result<PackageKey, Self::Error> {
        let s: &str = cursed::StrMarshaler::from_foreign(string)?;
        PackageKey::try_from(s).map_err(|e| Box::new(e) as _)
    }
}

#[cthulhu::invoke(return_marshaler = "cursed::UnitMarshaler")]
pub extern "C" fn pahkat_store_config_set_ui_value(
    #[marshal(cursed::ArcRefMarshaler::<RwLock<StoreConfig>>)] handle: Arc<RwLock<StoreConfig>>,
    #[marshal(cursed::StrMarshaler)] key: &str,
    #[marshal(cursed::StrMarshaler)] value: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    let config = handle.write().unwrap();
    config.set_ui_value(key, value.map(|x| x.to_string()))
}

#[cthulhu::invoke(return_marshaler = "cursed::StringMarshaler")]
pub extern "C" fn pahkat_store_config_ui_value(
    #[marshal(cursed::ArcRefMarshaler::<RwLock<StoreConfig>>)] handle: Arc<RwLock<StoreConfig>>,
    #[marshal(cursed::StrMarshaler)] key: &str,
) -> Option<String> {
    let config = handle.read().unwrap();
    config.ui_value(key)
}

#[cthulhu::invoke(return_marshaler = "cursed::StringMarshaler")]
pub extern "C" fn pahkat_store_config_skipped_package(
    #[marshal(cursed::ArcRefMarshaler::<RwLock<StoreConfig>>)] handle: Arc<RwLock<StoreConfig>>,
    #[marshal(PackageKeyMarshaler)] key: PackageKey,
) -> Option<String> {
    let config = handle.read().unwrap();
    config.skipped_package(&key)
}

#[cthulhu::invoke(return_marshaler = "cursed::UnitMarshaler")]
pub extern "C" fn pahkat_store_config_add_skipped_package(
    #[marshal(cursed::ArcRefMarshaler::<RwLock<StoreConfig>>)] handle: Arc<RwLock<StoreConfig>>,
    #[marshal(PackageKeyMarshaler)] key: PackageKey,
    #[marshal(cursed::StrMarshaler)] version: &str,
) -> Result<(), Box<dyn Error>> {
    let config = handle.read().unwrap();
    config.add_skipped_package(key, version.into())
}


#[cthulhu::invoke(return_marshaler = "JsonMarshaler")]
pub extern "C" fn pahkat_store_config_repos(
    #[marshal(cursed::ArcRefMarshaler::<RwLock<StoreConfig>>)] handle: Arc<RwLock<StoreConfig>>,
) -> Vec<RepoRecord> {
    let config = handle.read().unwrap();
    config.repos()
}

#[cthulhu::invoke(return_marshaler = "cursed::UnitMarshaler")]
pub extern "C" fn pahkat_store_config_set_repos(
    #[marshal(cursed::ArcRefMarshaler::<RwLock<StoreConfig>>)] handle: Arc<RwLock<StoreConfig>>,
    #[marshal(JsonMarshaler)] repos: Vec<RepoRecord>,
) -> Result<(), Box<dyn Error>> {
    handle.write().unwrap().set_repos(repos)
}

#[no_mangle]
pub extern "C" fn pahkat_str_free(ptr: *const libc::c_char) {
    if !ptr.is_null() {
        unsafe { CString::from_raw(ptr as *mut _) };
    }
}
