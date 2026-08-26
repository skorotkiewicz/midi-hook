use std::process::{Child, Command};

pub(super) struct ToggleProcess {
    child: Child,
    #[cfg(unix)]
    process_group: i32,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}

impl ToggleProcess {
    #[cfg(unix)]
    pub(super) fn spawn(mut command: Command) -> Result<Self, String> {
        use std::os::unix::process::CommandExt;

        command.process_group(0);
        let child = command.spawn().map_err(|error| error.to_string())?;
        let process_group = i32::try_from(child.id()).map_err(|_| "process ID is too large")?;
        Ok(Self {
            child,
            process_group,
        })
    }

    #[cfg(windows)]
    pub(super) fn spawn(mut command: Command) -> Result<Self, String> {
        use std::mem::{size_of, zeroed};
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };

        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(format!(
                    "could not create Windows Job Object: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let mut information: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &information as *const _ as *const _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
            {
                let error = std::io::Error::last_os_error();
                CloseHandle(job);
                return Err(format!("could not configure Windows Job Object: {error}"));
            }
            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(error) => {
                    CloseHandle(job);
                    return Err(error.to_string());
                }
            };
            if AssignProcessToJobObject(job, child.as_raw_handle() as _) == 0 {
                let error = std::io::Error::last_os_error();
                let _ = child.kill();
                let _ = child.wait();
                CloseHandle(job);
                return Err(format!(
                    "could not assign command to Windows Job Object: {error}"
                ));
            }
            Ok(Self { child, job })
        }
    }

    #[cfg(unix)]
    pub(super) fn is_running(&mut self) -> Result<bool, String> {
        self.child.try_wait().map_err(|error| error.to_string())?;
        Ok(process_group_exists(self.process_group))
    }

    #[cfg(windows)]
    pub(super) fn is_running(&mut self) -> Result<bool, String> {
        use std::mem::{size_of, zeroed};
        use windows_sys::Win32::System::JobObjects::{
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
            QueryInformationJobObject,
        };

        self.child.try_wait().map_err(|error| error.to_string())?;
        let mut information: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
        if unsafe {
            QueryInformationJobObject(
                self.job,
                JobObjectBasicAccountingInformation,
                &mut information as *mut _ as *mut _,
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(format!(
                "could not query Windows Job Object: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(information.ActiveProcesses > 0)
    }

    #[cfg(unix)]
    pub(super) fn stop(mut self) -> Result<(), String> {
        use std::time::{Duration, Instant};

        let group = -self.process_group;
        unsafe {
            libc::kill(group, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        while process_group_exists(self.process_group) && Instant::now() < deadline {
            let _ = self.child.try_wait();
            std::thread::sleep(Duration::from_millis(20));
        }
        if process_group_exists(self.process_group) {
            unsafe {
                libc::kill(group, libc::SIGKILL);
            }
        }
        self.child
            .wait()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    #[cfg(windows)]
    pub(super) fn stop(mut self) -> Result<(), String> {
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;

        if unsafe { TerminateJobObject(self.job, 1) } == 0 {
            return Err(format!(
                "could not terminate Windows Job Object: {}",
                std::io::Error::last_os_error()
            ));
        }
        self.child
            .wait()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

#[cfg(unix)]
fn process_group_exists(process_group: i32) -> bool {
    if unsafe { libc::kill(-process_group, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(windows)]
impl Drop for ToggleProcess {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.job);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn stops_unix_process_group() {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg("sleep 30 & wait");
        let process = ToggleProcess::spawn(command).unwrap();
        let process_group = process.process_group;
        assert!(process_group_exists(process_group));
        process.stop().unwrap();
        assert!(!process_group_exists(process_group));
    }

    #[cfg(unix)]
    #[test]
    fn recognizes_exited_unix_process() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "true"]);
        let mut process = ToggleProcess::spawn(command).unwrap();
        for _ in 0..100 {
            if !process.is_running().unwrap() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("process did not exit");
    }

    #[cfg(windows)]
    #[test]
    fn stops_windows_job() {
        let mut command = Command::new("cmd.exe");
        command.args(["/C", "ping -n 30 127.0.0.1 >NUL"]);
        ToggleProcess::spawn(command).unwrap().stop().unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn recognizes_exited_windows_process() {
        let mut command = Command::new("cmd.exe");
        command.args(["/C", "exit 0"]);
        let mut process = ToggleProcess::spawn(command).unwrap();
        for _ in 0..100 {
            if !process.is_running().unwrap() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("process did not exit");
    }
}
