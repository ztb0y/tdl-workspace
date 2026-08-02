use std::{fs::File, io, path::Path};

use anyhow::{Context, Result, bail};

use crate::{
    RELEASE_MANIFEST_URL,
    manifest::{ReleaseManifest, sha256_file},
};

pub fn fetch_release_manifest() -> Result<ReleaseManifest> {
    let response = ureq::get(RELEASE_MANIFEST_URL)
        .call()
        .context("fetch release manifest")?;
    let manifest: ReleaseManifest = serde_json::from_reader(response.into_body().into_reader())
        .context("decode release manifest")?;
    manifest.validate()?;
    Ok(manifest)
}

pub fn is_newer(current: &str, candidate: &str) -> Result<bool> {
    Ok(semver::Version::parse(candidate)? > semver::Version::parse(current)?)
}

pub fn download_setup(manifest: &ReleaseManifest, path: &Path) -> Result<()> {
    let partial = path.with_extension("exe.part");
    let result = (|| -> Result<()> {
        let url = manifest.setup_url()?;
        let mut response = ureq::get(&url)
            .call()
            .context("download setup")?
            .into_body()
            .into_reader();
        let mut output = File::create(&partial)?;
        io::copy(&mut response, &mut output)?;
        output.sync_all()?;
        if output.metadata()?.len() != manifest.setup.size {
            bail!("downloaded setup size mismatch");
        }
        drop(output);
        let actual = sha256_file(&partial)?;
        if !actual.eq_ignore_ascii_case(&manifest.setup.sha256) {
            bail!("downloaded setup checksum mismatch");
        }
        let _ = std::fs::remove_file(path);
        std::fs::rename(&partial, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_semver_versions() {
        assert!(is_newer("1.0.0", "1.1.0").unwrap());
        assert!(!is_newer("1.1.0", "1.1.0").unwrap());
    }
}
