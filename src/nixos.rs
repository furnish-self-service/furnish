//! How this NixOS computer is set up, for the NixOS card in the app's settings. Which packages its
//! configuration installs is the resource's business (resources/unix/furnish-nix.sh); this only
//! says where the configuration lives, so furnish can explain what it does with apps here.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Setup {
    /// The configuration on this computer: `/etc/nixos/flake.nix` or `configuration.nix`.
    /// `None`: not there, e.g. a fleet's configuration deployed from elsewhere.
    pub config: Option<PathBuf>,
    /// The configuration is a flake (else channels).
    pub flake: bool,
    /// This user has Home Manager (standalone or as a NixOS module).
    pub home_manager: bool,
}

/// `None` when this isn't NixOS.
pub fn setup() -> Option<Setup> {
    let user = std::env::var("USER").unwrap_or_default();
    detect(Path::new("/"), dirs::home_dir().as_deref(), &user)
}

fn detect(root: &Path, home: Option<&Path>, user: &str) -> Option<Setup> {
    if !root.join("etc/NIXOS").exists() {
        return None;
    }
    let etc = root.join("etc/nixos");
    let config = [etc.join("flake.nix"), etc.join("configuration.nix")].into_iter().find(|p| p.is_file());
    let flake = config.as_ref().is_some_and(|p| p.ends_with("flake.nix"));
    // Its generations: in the user's state folder, or (older Home Manager) in Nix's per-user profiles.
    let mut generations = vec![root.join("nix/var/nix/profiles/per-user").join(user).join("home-manager")];
    if let Some(home) = home {
        generations.push(home.join(".local/state/nix/profiles/home-manager"));
        generations.push(home.join(".config/home-manager"));
    }
    let home_manager = !user.is_empty() && generations.iter().any(|p| p.exists());
    Some(Setup { config, flake, home_manager })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_nixos_flake_channels_and_home_manager() {
        let root = std::env::temp_dir().join(format!("furnish-nixos-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home/ada");
        std::fs::create_dir_all(root.join("etc/nixos")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        assert_eq!(detect(&root, Some(&home), "ada"), None, "not NixOS without /etc/NIXOS");

        std::fs::write(root.join("etc/NIXOS"), "").unwrap();
        assert_eq!(detect(&root, Some(&home), "ada"), Some(Setup { config: None, flake: false, home_manager: false }));

        std::fs::write(root.join("etc/nixos/configuration.nix"), "{ }").unwrap();
        let s = detect(&root, Some(&home), "ada").unwrap();
        assert!(!s.flake && s.config.unwrap().ends_with("configuration.nix"));

        std::fs::write(root.join("etc/nixos/flake.nix"), "{ }").unwrap();
        std::fs::create_dir_all(home.join(".local/state/nix/profiles/home-manager")).unwrap();
        let s = detect(&root, Some(&home), "ada").unwrap();
        assert!(s.flake && s.home_manager && s.config.unwrap().ends_with("flake.nix"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
