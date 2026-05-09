use std::ffi::{CStr, CString, c_void};
use std::fmt::Debug;
use std::mem::{size_of, transmute, zeroed};
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, HWND, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, MODULEENTRY32, Module32First, Module32Next, PROCESSENTRY32,
            Process32First, Process32Next, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32,
            TH32CS_SNAPPROCESS,
        },
        LibraryLoader::{GetModuleHandleA, GetProcAddress},
        Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE, VirtualAllocEx},
        ProcessStatus::{
            EnumProcessModulesEx, GetModuleInformation, LIST_MODULES_64BIT, MODULEINFO,
        },
        Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_ALL_ACCESS, PROCESS_QUERY_INFORMATION,
            PROCESS_VM_OPERATION, PROCESS_VM_READ,
        },
    },
    UI::WindowsAndMessaging::{
        FindWindowA, FindWindowExA, GetWindowTextA, GetWindowThreadProcessId, IsWindowVisible,
    },
};

type NtReadVirtualMemory = unsafe extern "system" fn(
    process_handle: HANDLE,
    base_address: *const c_void,
    buffer: *mut c_void,
    number_of_bytes_to_read: u32,
    number_of_bytes_read: *mut u32,
) -> i32;

type NtWriteVirtualMemory = unsafe extern "system" fn(
    process_handle: HANDLE,
    base_address: *mut c_void,
    buffer: *const c_void,
    number_of_bytes_to_write: u32,
    number_of_bytes_written: *mut u32,
) -> i32;

#[derive(Clone)]
pub struct MemoryFunctions {
    pub nt_read: NtReadVirtualMemory,
    pub nt_write: NtWriteVirtualMemory,
}

impl MemoryFunctions {
    pub fn new() -> Option<Self> {
        unsafe {
            let ntdll = GetModuleHandleA(b"ntdll.dll\0".as_ptr());
            if ntdll == null_mut() {
                return None;
            }

            let nt_read_addr = GetProcAddress(ntdll, b"NtReadVirtualMemory\0".as_ptr())?;
            let nt_write_addr = GetProcAddress(ntdll, b"NtWriteVirtualMemory\0".as_ptr())?;

            Some(Self {
                nt_read: transmute(nt_read_addr),
                nt_write: transmute(nt_write_addr),
            })
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessModule {
    pub base: usize,
    pub size: usize,
}

pub struct Process {
    pub pid: u32,
    pub handle: HANDLE,
    pub hwnd: HWND,
    pub base_client: ProcessModule,
    mem_fns: MemoryFunctions,
}

impl Process {
    pub fn new() -> Option<Self> {
        MemoryFunctions::new().map(|fns| Self {
            pid: 0,
            handle: 0 as _,
            hwnd: 0 as _,
            base_client: ProcessModule { base: 0, size: 0 },
            mem_fns: fns,
        })
    }

    pub fn attach_process(&mut self, process_name: &str) -> bool {
        self.pid = find_pid_by_name(process_name);
        if self.pid == 0 {
            return false;
        }

        unsafe {
            self.handle = OpenProcess(
                PROCESS_QUERY_INFORMATION | PROCESS_VM_OPERATION | PROCESS_VM_READ,
                0,
                self.pid,
            );
            if self.handle == 0 as _ {
                return false;
            }

            let mut modules: [usize; 255] = [0; 255];
            let mut cb_needed = 0;
            if EnumProcessModulesEx(
                self.handle,
                modules.as_mut_ptr() as _,
                size_of::<[usize; 255]>() as u32,
                &mut cb_needed,
                LIST_MODULES_64BIT,
            ) != 0
            {
                self.base_client.base = modules[0];

                let mut module_info: MODULEINFO = zeroed();
                if GetModuleInformation(
                    self.handle,
                    modules[0] as _,
                    &mut module_info,
                    size_of::<MODULEINFO>() as u32,
                ) != 0
                {
                    self.base_client.size = module_info.SizeOfImage as usize;
                }
            }

            self.hwnd = get_window_handle_from_pid(self.pid);
        }
        true
    }

    pub fn attach_window(&mut self, window_name: &str) -> bool {
        self.pid = find_pid_by_window(window_name);
        if self.pid == 0 {
            return false;
        }

        unsafe {
            self.handle = OpenProcess(PROCESS_ALL_ACCESS, 0, self.pid);
            if self.handle == 0 as _ {
                return false;
            }

            let mut modules: [usize; 255] = [0; 255];
            let mut cb_needed = 0;
            if EnumProcessModulesEx(
                self.handle,
                modules.as_mut_ptr() as _,
                size_of::<[usize; 255]>() as u32,
                &mut cb_needed,
                LIST_MODULES_64BIT,
            ) != 0
            {
                self.base_client.base = modules[0];

                let mut module_info: MODULEINFO = zeroed();
                if GetModuleInformation(
                    self.handle,
                    modules[0] as _,
                    &mut module_info,
                    size_of::<MODULEINFO>() as u32,
                ) != 0
                {
                    self.base_client.size = module_info.SizeOfImage as usize;
                }
            }

            self.hwnd = get_window_handle_from_pid(self.pid);
        }
        true
    }

    pub fn update_hwnd(&mut self) -> bool {
        self.hwnd = get_window_handle_from_pid(self.pid);
        self.hwnd != 0 as _
    }

    pub fn close(&mut self) {
        if self.handle != 0 as _ {
            unsafe { CloseHandle(self.handle) };
            self.handle = 0 as _;
        }
    }

    pub fn is_alive(&self) -> bool {
        if self.handle == 0 as _ {
            return false;
        }
        unsafe {
            let mut exit_code = 0;
            if GetExitCodeProcess(self.handle, &mut exit_code) != 0 {
                return exit_code == 259;
            }
        }
        false
    }

    pub fn get_module(&self, module_name: &str) -> ProcessModule {
        find_module(self.pid, module_name)
    }

    pub fn allocate(&self, size: usize) -> *mut c_void {
        unsafe {
            VirtualAllocEx(
                self.handle,
                null(),
                size,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        }
    }

    pub fn read_raw(&self, address: usize, buffer: *mut c_void, size: usize) -> bool {
        let mut bytes_read = 0;
        unsafe {
            if (self.mem_fns.nt_read as usize) == 0 {
                return false;
            }
            let status = (self.mem_fns.nt_read)(
                self.handle,
                address as _,
                buffer,
                size as u32,
                &mut bytes_read,
            );
            status == 0 || bytes_read == size as u32
        }
    }

    pub fn write_raw(&self, address: usize, buffer: *const c_void, size: usize) -> bool {
        let mut bytes_written = 0;
        unsafe {
            let status = (self.mem_fns.nt_write)(
                self.handle,
                address as _,
                buffer,
                size as u32,
                &mut bytes_written,
            );
            status == 0 || bytes_written == size as u32
        }
    }

    pub fn read<T: Copy + Default>(&self, address: usize) -> T {
        let mut buffer = T::default();
        self.read_raw(address, &mut buffer as *mut T as _, size_of::<T>());
        buffer
    }

    pub fn write<T: Copy>(&self, address: usize, value: T) {
        self.write_raw(address, &value as *const T as _, size_of::<T>());
    }

    pub fn write_bytes(&self, address: usize, bytes: &[u8]) {
        self.write_raw(address, bytes.as_ptr() as _, bytes.len());
    }

    pub fn read_multi_address(&self, base: usize, offsets: &[usize]) -> usize {
        let mut addr = base;
        for &offset in offsets {
            addr = self.read::<usize>(addr + offset);
        }
        addr
    }

    pub fn read_multi<T: Copy + Default>(&self, base: usize, offsets: &[usize]) -> T {
        if offsets.is_empty() {
            return T::default();
        }
        let mut addr = base;
        for i in 0..offsets.len() - 1 {
            addr = self.read::<usize>(addr + offsets[i]);
        }
        self.read::<T>(addr + offsets.last().unwrap())
    }

    pub fn find_signature(&self, signature: &[Option<u8>]) -> usize {
        self.find_signature_in_module(self.base_client.base, self.base_client.size, signature)
    }

    pub fn find_signature_in_module(
        &self,
        base: usize,
        size: usize,
        signature: &[Option<u8>],
    ) -> usize {
        if size == 0 {
            return 0;
        }

        let mut data = vec![0u8; size];

        if !self.read_raw(base, data.as_mut_ptr() as _, size) {
            return 0;
        }

        for i in 0..size.saturating_sub(signature.len()) {
            let mut found = true;
            for j in 0..signature.len() {
                if let Some(byte) = signature[j] {
                    if data[i + j] != byte {
                        found = false;
                        break;
                    }
                }
            }
            if found {
                return base + i;
            }
        }
        0
    }

    pub fn read_offset_from_module<T>(
        &self,
        module: ProcessModule,
        signature: &[Option<u8>],
        offset: usize,
    ) -> usize
    where
        T: Copy + Default + TryInto<isize>,
        <T as TryInto<isize>>::Error: Debug,
    {
        let addr = self.find_signature_in_module(module.base, module.size, signature);
        if addr == 0 {
            return 0;
        }

        let offset_val: T = self.read(addr + offset);
        let offset_isize: isize = offset_val.try_into().unwrap_or(0);
        (addr as isize + offset_isize + offset as isize + size_of::<T>() as isize) as usize
    }

    pub fn read_offset_from_signature<T>(&self, signature: &[Option<u8>], offset: usize) -> usize
    where
        T: Copy + Default + TryInto<isize>,
        <T as TryInto<isize>>::Error: Debug,
    {
        self.read_offset_from_module::<T>(self.base_client, signature, offset)
    }
}

pub fn parse_signature(sig: &str) -> Vec<Option<u8>> {
    sig.split_whitespace()
        .map(|s| {
            if s == "?" || s == "??" {
                None
            } else {
                Some(u8::from_str_radix(s, 16).unwrap_or(0))
            }
        })
        .collect()
}

fn find_pid_by_name(name: &str) -> u32 {
    let name_lower = name.to_ascii_lowercase();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE as _ {
            return 0;
        }

        let mut entry: PROCESSENTRY32 = zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32>() as u32;

        if Process32First(snapshot, &mut entry) != 0 {
            loop {
                let entry_name = CStr::from_ptr(entry.szExeFile.as_ptr() as _)
                    .to_string_lossy()
                    .to_ascii_lowercase();

                if entry_name == name_lower {
                    CloseHandle(snapshot);
                    return entry.th32ProcessID;
                }

                if Process32Next(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
    }
    0
}

fn find_pid_by_window(name: &str) -> u32 {
    unsafe {
        let c_name = CString::new(name).unwrap_or_default();
        let hwnd = FindWindowA(null(), c_name.as_ptr() as _);
        if hwnd == 0 as _ {
            return 0;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        pid
    }
}

fn get_window_handle_from_pid(pid: u32) -> HWND {
    unsafe {
        let mut hwnd = 0 as _;
        loop {
            hwnd = FindWindowExA(0 as _, hwnd, null(), null());
            if hwnd == 0 as _ {
                break;
            }
            let mut window_pid = 0;
            GetWindowThreadProcessId(hwnd, &mut window_pid);
            if window_pid == pid {
                let mut title: [u8; 260] = [0; 260];
                GetWindowTextA(hwnd, title.as_mut_ptr(), 260);
                if IsWindowVisible(hwnd) != 0 && title[0] != 0 {
                    return hwnd;
                }
            }
        }
    }
    null_mut()
}

fn find_module(pid: u32, name: &str) -> ProcessModule {
    let name_lower = name.to_ascii_lowercase();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);
        if snapshot == INVALID_HANDLE_VALUE as _ {
            return ProcessModule { base: 0, size: 0 };
        }

        let mut entry: MODULEENTRY32 = zeroed();
        entry.dwSize = size_of::<MODULEENTRY32>() as u32;

        if Module32First(snapshot, &mut entry) != 0 {
            loop {
                let entry_name = CStr::from_ptr(entry.szModule.as_ptr() as _)
                    .to_string_lossy()
                    .to_ascii_lowercase();

                if entry_name == name_lower {
                    CloseHandle(snapshot);
                    return ProcessModule {
                        base: entry.modBaseAddr as usize,
                        size: entry.modBaseSize as usize,
                    };
                }

                if Module32Next(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
    }
    ProcessModule { base: 0, size: 0 }
}
