//! Paths between the shell's machine and the backend's, which differ when the backend runs in WSL.

/// The backend-side path of a folder the native dialog returned. In the WSL split the
/// dialog shows `\\wsl.localhost\<distro>\…` (or `\\wsl$\<distro>\…`), which is the Linux
/// path `/…` inside that distro; anything else cannot be opened by the backend.
pub fn backend_path(picked: &str, distro: Option<&str>) -> Result<String, String> {
    let Some(distro) = distro else {
        return Ok(picked.to_owned());
    };
    let normalized = picked.replace('\\', "/");
    let rest = normalized
        .strip_prefix("//wsl.localhost/")
        .or_else(|| normalized.strip_prefix("//wsl$/"));
    match rest {
        Some(rest) => {
            let (from, path) = rest.split_once('/').unwrap_or((rest, ""));
            if from.eq_ignore_ascii_case(distro) {
                Ok(format!("/{path}"))
            } else {
                Err(format!(
                    "{picked} is in the WSL distro {from}, but the backend runs in {distro}; pick a folder inside {distro}"
                ))
            }
        }
        None => Err(format!(
            "{picked} is not inside the WSL distro {distro}, where the backend runs (a Windows drive is not visible to it); pick a folder under \\\\wsl.localhost\\{distro}"
        )),
    }
}

/// `path`, a path of the machine the backend runs on, as this machine names it: in WSL, the
/// share of the distro.
pub fn host_path(path: &str, distro: Option<&str>) -> String {
    match distro {
        Some(distro) => format!(r"\\wsl.localhost\{distro}{}", path.replace('/', r"\")),
        None => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{backend_path, host_path};

    #[test]
    fn a_folder_in_wsl_is_named_by_its_share_and_one_on_this_machine_is_left_alone() {
        assert_eq!(
            host_path("/home/me/.local/share/vet/logs", Some("NixOS")),
            r"\\wsl.localhost\NixOS\home\me\.local\share\vet\logs"
        );
        assert_eq!(
            host_path("/Users/me/Library/Application Support/vet/logs", None),
            "/Users/me/Library/Application Support/vet/logs"
        );
    }

    #[test]
    fn wsl_paths_become_linux_paths() {
        let wsl = Some("NixOS");
        assert_eq!(
            backend_path(r"\\wsl.localhost\NixOS\home\me\vet", wsl).as_deref(),
            Ok("/home/me/vet")
        );
        assert_eq!(backend_path(r"\\wsl$\nixos\home\me", wsl).as_deref(), Ok("/home/me"));
        assert_eq!(backend_path(r"\\wsl.localhost\NixOS", wsl).as_deref(), Ok("/"));
        assert_eq!(backend_path("/home/me", None).as_deref(), Ok("/home/me"));
    }

    #[test]
    fn paths_the_wsl_backend_cannot_see_are_refused() {
        let wsl = Some("NixOS");
        let other = backend_path(r"\\wsl.localhost\Ubuntu\home", wsl).unwrap_err();
        assert!(other.contains("Ubuntu") && other.contains("NixOS"), "{other}");
        let drive = backend_path(r"C:\src\vet", wsl).unwrap_err();
        assert!(drive.contains("Windows drive"), "{drive}");
    }
}
