use std::sync::{atomic, Arc, RwLock};
use std::collections::HashMap;
use std::fs::create_dir_all;

use super::*;
use ::{Repository, StoreConfig, PackageStatus, Download};
use ::macos::*;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PackageStatusResponse {
	pub status: PackageStatus,
    pub target: MacOSInstallTarget
}

fn parse_target(number: u8) -> MacOSInstallTarget {
	if number == 0 {
		MacOSInstallTarget::System
	} else {
		MacOSInstallTarget::User
	}
}

#[derive(Default)]
pub struct RpcImpl {
	pub uid: atomic::AtomicUsize,
	pub active: Arc<RwLock<HashMap<SubscriptionId, pubsub::Sink<String>>>>,
	pub repo_configs: Arc<RwLock<Vec<RepoConfig>>>,
	pub repo: Arc<RwLock<HashMap<String, Repository>>>
}

impl Rpc for RpcImpl {
	type Metadata = Meta;

	fn repository(&self, url: String, _channel: String) -> Result<Repository> {
		let repo = Repository::from_url(&url).unwrap();
		let mut repo_map = self.repo.write().unwrap();
		repo_map.insert(url, repo.clone());
		// println!("{:?}", repo);
		Ok(repo)
	}

	fn status(&self, repo_id: String, package_id: String, target: u8) -> Result<PackageStatus> {
		let repo = repo_check(&self, repo_id)?;
		let package = parse_package(&repo, &package_id)?;
		let target = parse_target(target);
		
		let config = StoreConfig::load_or_default();
		let store = MacOSPackageStore::new(&repo, &config);
		let status = store.status(&package, target);
		status.map_err(|e| {
			Error {
				code: ErrorCode::InvalidParams,
				message: format!("{}", e),
				data: None
			}
		})
	}

	fn repository_statuses(&self, repo_id: String) -> Result<BTreeMap<String, PackageStatusResponse>> {
		let repo = repo_check(&self, repo_id)?;
		
		let config = StoreConfig::load_or_default();
		let store = MacOSPackageStore::new(&repo, &config);

		let mut map = BTreeMap::new();

		for package in repo.packages().values() {
			let status = match store.status(&package, MacOSInstallTarget::System) {
				Ok(v) => v,
				Err(e) => {
					eprintln!("{:?}", e);
					PackageStatus::NotInstalled
				}
 			};

			match status {
				PackageStatus::NotInstalled => {},
				_ => {
					map.insert(package.id.clone(), PackageStatusResponse {
						status: status,
						target: MacOSInstallTarget::System
					});
					continue;
				}
			};

			let status = match store.status(&package, MacOSInstallTarget::User) {
				Ok(v) => v,
				Err(e) => {
					eprintln!("{:?}", e);
					PackageStatus::NotInstalled
				}
 			};

			map.insert(package.id.clone(), PackageStatusResponse {
				status: status,
				target: MacOSInstallTarget::User
			});
		}
		
		Ok(map)
	}

	fn install(&self, repo_id: String, package_id: String, target: u8) -> Result<PackageStatus> {
		let repo = repo_check(&self, repo_id)?;
		let package = parse_package(&repo, &package_id)?;
		let target = parse_target(target);
		
		let config = StoreConfig::load_or_default();
		let store = MacOSPackageStore::new(&repo, &config);
		store.install(&package, target).map_err(|e| {
			let msg = match e {
				MacOSInstallError::InstallerFailure(error) => {
					match error {
						ProcessError::Unknown(output) => String::from_utf8_lossy(&output.stdout).to_string(),
						_ => format!("{:?}", &error)
					}
				}
				_ => format!("{:?}", &e)
			};
			Error {
				code: ErrorCode::InvalidParams,
				message: msg,
				data: None
			}
		})
	}

	fn uninstall(&self, repo_id: String, package_id: String, target: u8) -> Result<PackageStatus> {
		let repo = repo_check(&self, repo_id)?;
		let package = parse_package(&repo, &package_id)?;
		let target = parse_target(target);
		
		let config = StoreConfig::load_or_default();
		let store = MacOSPackageStore::new(&repo, &config);
		
		store.uninstall(&package, target).map_err(|e| {
			let msg = match e {
				MacOSUninstallError::PkgutilFailure(error) => {
					match error {
						ProcessError::Unknown(output) => String::from_utf8_lossy(&output.stderr).to_string(),
						_ => format!("{:?}", &error)
					}
				}
				_ => format!("{:?}", &e)
			};

			Error {
				code: ErrorCode::InvalidParams,
				message: msg,
				data: None
			}
		})
	}

	fn download_subscribe(&self, _meta: Self::Metadata, subscriber: pubsub::Subscriber<[usize; 2]>, repo_id: String, package_id: String, target: u8) {
		let repo = match repo_check(&self, repo_id) {
			Ok(v) => v,
			Err(e) => {
				subscriber.reject(e).unwrap();
				return;
			}
		};

		let package = match parse_package(&repo, &package_id) {
			Ok(v) => v,
			Err(e) => {
				subscriber.reject(e).unwrap();
				return;
			}
		};

		let _target = parse_target(target);

		let id = self.uid.fetch_add(1, atomic::Ordering::SeqCst);
		let sub_id = SubscriptionId::Number(id as u64);
		let sink = subscriber.assign_id(sub_id.clone()).unwrap();

		thread::spawn(move || {
			let sink = sink.clone();
			let config = StoreConfig::load_or_default();
			let store = MacOSPackageStore::new(&repo, &config);

			let package_cache = store.download_path(&package);
			// println!("{:?}", &package_cache);
			if !package_cache.exists() {
				create_dir_all(&package_cache).unwrap();
			}

			let pkg_path = package.download(&package_cache, 
				Some(|cur, max| {
					match sink.notify(Ok([cur, max])).wait() {
						Ok(_) => {},
						Err(_) => {}
					}
				}));
			
			match pkg_path {
				Ok(_) => {},
				Err(e) => {
					eprintln!("{:?}", &e);
					let error = Error {
						code: ErrorCode::InvalidParams,
						message: format!("{:?}", &e),
						data: None
					};
					sink.notify(Err(error)).wait().expect("Wait maybe, crash never.");
				}
			};
		});
	}

	fn download_unsubscribe(&self, _id: SubscriptionId) -> Result<bool> {
		// TODO: handle cancel request
		return Ok(true)
	}
}
