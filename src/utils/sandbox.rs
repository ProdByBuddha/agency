//! Sandbox Utilities (Seatbelt)
//! 
//! Centralizes macOS Seatbelt (sandbox-exec) policies and helpers.

pub const TOOL_SANDBOX_POLICY: &str = r#"
(version 1)
(deny default)
(import "system.sb")

(allow process-exec)
(allow process-fork)

;; Allow reading system libs
(allow file-read* (subpath "/usr/lib"))
(allow file-read* (subpath "/usr/share"))
(allow file-read* (subpath "/System/Library"))

;; Allow reading/writing to /tmp and the current directory (Workspace)
(allow file-read* file-write* (subpath "/private/tmp"))
(allow file-read* file-write* (subpath "/var/folders"))
(allow file-read* file-write* (subpath (param "WORKSPACE_DIR")))

;; Allow execution of common compilers and runtimes
(allow file-read* (subpath "/usr/bin"))
(allow file-read* (subpath "/usr/local/bin"))
(allow file-read* (subpath "/opt/homebrew/bin"))

;; Allow network-outbound for package managers/scripts
(allow network-outbound)

(allow sysctl-read)
"#;
