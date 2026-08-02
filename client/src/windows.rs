use std::{
    env,
    ffi::{OsStr, OsString, c_void},
    fs, io,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
    process::{Child, Command},
    ptr,
    time::Duration,
};

use ::windows::{
    Win32::{
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoTaskMemFree, CoUninitialize, IPersistFile,
        },
        UI::Shell::{
            FOLDERID_Desktop, FOLDERID_Programs, IShellLinkW, KF_FLAG_DEFAULT,
            SHGetKnownFolderPath, ShellLink,
        },
    },
    core::{GUID, Interface, PCWSTR},
};
use anyhow::{Context, Result, bail};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_TIMEOUT},
    Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        },
        Threading::CreateMutexW,
    },
    UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
};
use winreg::{RegKey, enums::HKEY_CURRENT_USER};

pub struct SingleInstance(HANDLE);

impl SingleInstance {
    pub fn acquire(name: &str) -> Result<Self> {
        let name = wide(name);
        let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error()).context("create launcher mutex");
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(handle) };
            bail!("tdl-web launcher is already running");
        }
        Ok(Self(handle))
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

pub struct Job(HANDLE);

unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Job {
    pub fn new() -> Result<Self> {
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error()).context("create process job");
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const c_void,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            unsafe { CloseHandle(handle) };
            return Err(io::Error::last_os_error()).context("configure process job");
        }
        Ok(Self(handle))
    }

    pub fn assign(&self, child: &Child) -> Result<()> {
        let handle = child.as_raw_handle() as HANDLE;
        if unsafe { AssignProcessToJobObject(self.0, handle) } == 0 {
            return Err(io::Error::last_os_error()).context("assign process job");
        }
        Ok(())
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

pub fn open_browser(url: &str) -> Result<()> {
    let operation = wide("open");
    let url = wide(url);
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            operation.as_ptr(),
            url.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result <= 32 {
        bail!("open browser failed with code {result}");
    }
    Ok(())
}

pub fn wait_for_pid(pid: u32, timeout: Duration) -> Result<()> {
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject};

    const SYNCHRONIZE_PROCESS: u32 = 0x0010_0000;
    let handle = unsafe { OpenProcess(SYNCHRONIZE_PROCESS, 0, pid) };
    if handle.is_null() {
        return Ok(());
    }
    let result =
        unsafe { WaitForSingleObject(handle, timeout.as_millis().min(u32::MAX as u128) as u32) };
    unsafe { CloseHandle(handle) };
    if result == WAIT_TIMEOUT {
        bail!("timed out waiting for process {pid}");
    }
    Ok(())
}

pub fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    let from = wide(from);
    let to = wide(to);
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn register_install(root: &Path, version: &str) -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) =
        hkcu.create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\tdl-web")?;
    let launcher = root.join("tdl-web-launcher.exe");
    let uninstaller = root.join("tdl-web-uninstaller.exe");
    key.set_value("DisplayName", &"tdl-web")?;
    key.set_value("DisplayVersion", &version)?;
    key.set_value("Publisher", &"ztb0y")?;
    key.set_value("InstallLocation", &root.to_string_lossy().as_ref())?;
    key.set_value("DisplayIcon", &launcher.to_string_lossy().as_ref())?;
    key.set_value("UninstallString", &format!("\"{}\"", uninstaller.display()))?;
    key.set_value("NoModify", &1_u32)?;
    key.set_value("NoRepair", &1_u32)?;
    create_shortcuts(root)
}

pub fn unregister_install() -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let _ =
        hkcu.delete_subkey_all("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\tdl-web");
    remove_shortcuts()
}

fn create_shortcuts(root: &Path) -> Result<()> {
    let _com = Com::initialize()?;
    let launcher = root.join("tdl-web-launcher.exe");
    for folder in [&FOLDERID_Desktop, &FOLDERID_Programs] {
        create_shortcut(&known_folder(folder)?.join("tdl-web.lnk"), &launcher, root)?;
    }
    Ok(())
}

fn remove_shortcuts() -> Result<()> {
    for folder in [&FOLDERID_Desktop, &FOLDERID_Programs] {
        let path = known_folder(folder)?.join("tdl-web.lnk");
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("remove {}", path.display())),
        }
    }
    Ok(())
}

fn create_shortcut(path: &Path, target: &Path, working_directory: &Path) -> Result<()> {
    let target = wide(target);
    let working_directory = wide(working_directory);
    let path = wide(path);
    let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
        .context("create shell link")?;
    unsafe {
        link.SetPath(PCWSTR(target.as_ptr()))?;
        link.SetWorkingDirectory(PCWSTR(working_directory.as_ptr()))?;
        link.SetIconLocation(PCWSTR(target.as_ptr()), 0)?;
    }
    let persist: IPersistFile = link.cast()?;
    unsafe { persist.Save(PCWSTR(path.as_ptr()), true) }.context("save shell link")?;
    Ok(())
}

fn known_folder(id: &GUID) -> Result<PathBuf> {
    let value = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }
        .context("resolve Windows known folder")?;
    let path = PathBuf::from(OsString::from_wide(unsafe { value.as_wide() }));
    unsafe { CoTaskMemFree(Some(value.as_ptr().cast())) };
    Ok(path)
}

struct Com;

impl Com {
    fn initialize() -> Result<Self> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok() }.context("initialize COM")?;
        Ok(Self)
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

pub fn schedule_self_delete(path: &Path) -> Result<()> {
    let command = format!(
        "ping 127.0.0.1 -n 2 >nul & del /F /Q \"{}\"",
        path.display()
    );
    Command::new("cmd.exe")
        .args(["/D", "/C", &command])
        .spawn()
        .context("schedule uninstaller cleanup")?;
    Ok(())
}

pub fn local_temp_dir() -> std::path::PathBuf {
    env::temp_dir().join("tdl-web")
}

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}
