use std::ffi::OsString;

#[derive(Debug, PartialEq, Eq)]
pub enum LaunchMode {
    Runtime,
    #[cfg(feature = "ui-preview")]
    Preview {
        fail: bool,
    },
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<LaunchMode, String> {
    let mut args = args.into_iter();
    let Some(arg) = args.next() else {
        #[cfg(feature = "ui-preview")]
        return Ok(LaunchMode::Preview { fail: false });

        #[cfg(not(feature = "ui-preview"))]
        return Ok(LaunchMode::Runtime);
    };
    if args.next().is_some() {
        return Err("Pass just one option: --runtime, --preview, or --preview-failure.".into());
    }

    match arg.to_str() {
        Some("--runtime") => Ok(LaunchMode::Runtime),
        Some("--preview" | "--preview-failure") => {
            #[cfg(feature = "ui-preview")]
            return Ok(LaunchMode::Preview {
                fail: arg == "--preview-failure",
            });

            #[cfg(not(feature = "ui-preview"))]
            Err("Preview is not included in this build. Run cargo run -p simp-installer --features ui-preview -- --preview.".into())
        }
        _ => Err(format!("Unknown option: {}", arg.to_string_lossy())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(feature = "ui-preview"))]
    #[test]
    fn normal_launch_never_selects_preview() {
        assert_eq!(parse([]), Ok(LaunchMode::Runtime));
    }

    #[test]
    fn rejects_unknown_and_conflicting_options() {
        assert!(parse(["--unknown".into()]).is_err());
        assert!(parse(["--preview".into(), "--preview-failure".into()]).is_err());
        assert_eq!(parse(["--runtime".into()]), Ok(LaunchMode::Runtime));
    }

    #[cfg(not(feature = "ui-preview"))]
    #[test]
    fn normal_build_rejects_preview_options() {
        assert!(parse(["--preview".into()]).is_err());
        assert!(parse(["--preview-failure".into()]).is_err());
    }

    #[cfg(feature = "ui-preview")]
    #[test]
    fn preview_is_default_with_feature() {
        assert_eq!(parse([]), Ok(LaunchMode::Preview { fail: false }));
        assert_eq!(
            parse(["--preview".into()]),
            Ok(LaunchMode::Preview { fail: false })
        );
        assert_eq!(
            parse(["--preview-failure".into()]),
            Ok(LaunchMode::Preview { fail: true })
        );
    }
}
