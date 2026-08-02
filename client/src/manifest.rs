use std::{
    fs::File,
    io::{BufReader, Read},
    path::{Component as PathComponent, Path},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Component {
    pub name: String,
    pub version: String,
    pub commit: String,
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ComponentsManifest {
    pub schema_version: u32,
    pub version: String,
    pub platform: String,
    pub components: Vec<Component>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SetupAsset {
    pub filename: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReleaseManifest {
    pub schema_version: u32,
    pub version: String,
    pub platform: String,
    pub setup: SetupAsset,
    pub components: Vec<Component>,
}

impl ComponentsManifest {
    pub fn load(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let manifest: Self = serde_json::from_reader(BufReader::new(file))
            .with_context(|| format!("decode {}", path.display()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            bail!(
                "unsupported component manifest schema {}",
                self.schema_version
            );
        }
        if self.platform != "windows-x64" {
            bail!("unsupported component platform {}", self.platform);
        }
        semver::Version::parse(&self.version).context("parse component manifest version")?;
        for component in &self.components {
            validate_relative_path(&component.path)?;
            validate_sha256(&component.sha256)?;
            validate_commit(&component.commit)?;
            if component.version != self.version {
                bail!("component version mismatch: {}", component.path);
            }
        }
        Ok(())
    }
}

impl ReleaseManifest {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            bail!(
                "unsupported release manifest schema {}",
                self.schema_version
            );
        }
        if self.platform != "windows-x64" {
            bail!("unsupported release platform {}", self.platform);
        }
        semver::Version::parse(&self.version).context("parse release version")?;
        if self.setup.filename != "tdl-web-setup-windows-x64.exe" {
            bail!("unexpected setup filename {}", self.setup.filename);
        }
        validate_sha256(&self.setup.sha256)?;
        for component in &self.components {
            validate_relative_path(&component.path)?;
            validate_sha256(&component.sha256)?;
            validate_commit(&component.commit)?;
            if component.version != self.version {
                bail!("component version mismatch: {}", component.path);
            }
        }
        Ok(())
    }

    pub fn setup_url(&self) -> Result<String> {
        self.validate()?;
        let version = semver::Version::parse(&self.version).context("parse release version")?;
        Ok(format!(
            "https://github.com/ztb0y/tdl-workspace/releases/download/v{version}/{}",
            self.setup.filename
        ))
    }
}

pub fn verify_components(root: &Path, manifest: &ComponentsManifest) -> Result<()> {
    manifest.validate()?;
    for component in &manifest.components {
        let path = root.join(&component.path);
        let metadata = path
            .metadata()
            .with_context(|| format!("missing component {}", component.path))?;
        if metadata.len() != component.size {
            bail!("component size mismatch: {}", component.path);
        }
        let actual = sha256_file(&path)?;
        if !actual.eq_ignore_ascii_case(&component.sha256) {
            bail!("component checksum mismatch: {}", component.path);
        }
    }
    Ok(())
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn validate_relative_path(path: &str) -> Result<()> {
    let path = Path::new(path);
    if path.as_os_str().is_empty() || path.is_absolute() {
        bail!("component path must be relative");
    }
    if path
        .components()
        .any(|part| !matches!(part, PathComponent::Normal(_)))
    {
        bail!("unsafe component path: {}", path.display());
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("invalid SHA-256 value");
    }
    Ok(())
}

fn validate_commit(value: &str) -> Result<()> {
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("invalid component commit");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_component_paths() {
        assert!(validate_relative_path("runtime/tdl.exe").is_ok());
        assert!(validate_relative_path("../tdl.exe").is_err());
        assert!(validate_relative_path("C:\\tdl.exe").is_err());
    }

    #[test]
    fn rejects_invalid_hashes_and_commits() {
        assert!(validate_sha256(&"0".repeat(64)).is_ok());
        assert!(validate_sha256("not-a-hash").is_err());
        assert!(validate_commit(&"a".repeat(40)).is_ok());
        assert!(validate_commit("workspace").is_err());
    }

    #[test]
    fn release_url_is_fixed_to_workspace_releases() {
        let manifest = ReleaseManifest {
            schema_version: 1,
            version: "1.2.3".into(),
            platform: "windows-x64".into(),
            setup: SetupAsset {
                filename: "tdl-web-setup-windows-x64.exe".into(),
                size: 1,
                sha256: "0".repeat(64),
            },
            components: vec![],
        };
        assert_eq!(
            manifest.setup_url().unwrap(),
            "https://github.com/ztb0y/tdl-workspace/releases/download/v1.2.3/tdl-web-setup-windows-x64.exe"
        );
    }
}
