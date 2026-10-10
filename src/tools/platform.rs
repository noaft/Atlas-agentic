use std::{process::Command, sync::Arc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    MacOs,
    Windows,
}

impl Platform {
    pub fn current() -> Self {
        #[cfg(target_os = "macos")]
        return Self::MacOs;
        #[cfg(windows)]
        return Self::Windows;
        #[cfg(all(unix, not(target_os = "macos")))]
        return Self::Linux;
        #[allow(unreachable_code)]
        Self::Linux
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
}

impl CommandSpec {
    pub fn new<I, S>(program: impl Into<String>, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }
}

pub fn file_command(platform: Platform, path: &str) -> CommandSpec {
    match platform {
        Platform::Linux => CommandSpec::new("xdg-open", [path]),
        Platform::MacOs => CommandSpec::new("open", [path]),
        Platform::Windows => CommandSpec::new("ShellExecuteW", [path]),
    }
}

pub fn application_command(platform: Platform, application: &str, args: &[String]) -> CommandSpec {
    match platform {
        Platform::MacOs => {
            let mut command_args = vec!["-a".to_string(), application.to_string()];
            if !args.is_empty() {
                command_args.push("--args".to_string());
                command_args.extend(args.iter().cloned());
            }
            CommandSpec::new("open", command_args)
        }
        Platform::Linux | Platform::Windows => CommandSpec::new(application, args.iter().cloned()),
    }
}

pub trait ProcessLauncher: Send + Sync {
    fn launch(&self, command: &CommandSpec) -> Result<(), String>;

    fn launch_file(&self, platform: Platform, path: &str) -> Result<(), String> {
        self.launch(&file_command(platform, path))
    }
}

pub struct SystemProcessLauncher;

impl ProcessLauncher for SystemProcessLauncher {
    fn launch(&self, command: &CommandSpec) -> Result<(), String> {
        let mut child = Command::new(&command.program)
            .args(&command.args)
            .spawn()
            .map_err(|error| format!("failed to launch {}: {error}", command.program))?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }

    fn launch_file(&self, platform: Platform, path: &str) -> Result<(), String> {
        #[cfg(windows)]
        if platform == Platform::Windows {
            return shell_execute(path);
        }
        self.launch(&file_command(platform, path))
    }
}

#[cfg(windows)]
fn shell_execute(path: &str) -> Result<(), String> {
    use std::{iter, ptr};
    use windows_sys::Win32::{UI::Shell::ShellExecuteW, UI::WindowsAndMessaging::SW_SHOWNORMAL};

    let path = path.encode_utf16().chain(iter::once(0)).collect::<Vec<_>>();
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            ptr::null(),
            path.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        Err(format!(
            "failed to open file with ShellExecuteW: code {}",
            result as isize
        ))
    } else {
        Ok(())
    }
}

pub fn system_launcher() -> Arc<dyn ProcessLauncher> {
    Arc::new(SystemProcessLauncher)
}
