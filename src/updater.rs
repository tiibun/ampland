use crate::error::AppError;
use crate::manifest::Target;

fn asset_name_for_target(platform: &str, arch: &str) -> Result<String, AppError> {
    match (platform, arch) {
        ("macos", "arm64") => Ok("ampland-macos-arm64".to_string()),
        ("macos", "x64") => Ok("ampland-macos-x64".to_string()),
        ("linux", "x64") => Ok("ampland-linux-x64".to_string()),
        ("windows", "x64") => Ok("ampland-windows-x64.exe".to_string()),
        (p, a) => Err(AppError::Other {
            message: format!("no release asset for platform={p} arch={a}"),
        }),
    }
}

pub fn asset_name_for_current_target() -> Result<String, AppError> {
    let t = Target::current()?;
    asset_name_for_target(&t.platform, &t.arch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_name_macos_arm64() {
        assert_eq!(
            asset_name_for_target("macos", "arm64").expect("ok"),
            "ampland-macos-arm64"
        );
    }

    #[test]
    fn asset_name_macos_x64() {
        assert_eq!(
            asset_name_for_target("macos", "x64").expect("ok"),
            "ampland-macos-x64"
        );
    }

    #[test]
    fn asset_name_linux_x64() {
        assert_eq!(
            asset_name_for_target("linux", "x64").expect("ok"),
            "ampland-linux-x64"
        );
    }

    #[test]
    fn asset_name_windows_x64() {
        assert_eq!(
            asset_name_for_target("windows", "x64").expect("ok"),
            "ampland-windows-x64.exe"
        );
    }

    #[test]
    fn asset_name_unknown_platform_errors() {
        assert!(asset_name_for_target("freebsd", "x64").is_err());
    }
}
