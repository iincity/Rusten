use windows_sys::Win32::{
    Foundation::CloseHandle,
    System::{
        ProcessStatus::EnumProcesses,
        Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW},
    },
};

pub fn running_process_names() -> Option<Vec<String>> {
    let mut ids = vec![0u32; 1024];
    loop {
        let mut bytes = 0u32;
        if unsafe { EnumProcesses(ids.as_mut_ptr(), (ids.len() * 4) as u32, &mut bytes) } == 0 {
            return None;
        }
        if bytes as usize == ids.len() * 4 {
            ids.resize(ids.len() * 2, 0);
        } else {
            ids.truncate(bytes as usize / 4);
            break;
        }
    }

    let mut names = Vec::new();
    let mut path = vec![0u16; 32_768];
    for id in ids.into_iter().filter(|id| *id != 0) {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, id) };
        if handle.is_null() {
            continue;
        }
        let mut length = path.len() as u32;
        if unsafe { QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut length) } != 0 {
            let image = String::from_utf16_lossy(&path[..length as usize]);
            if let Some(name) = image.rsplit(['\\', '/']).next() {
                names.push(name.to_owned());
            }
        }
        unsafe { CloseHandle(handle) };
    }
    (!names.is_empty()).then_some(names)
}

#[cfg(test)]
mod tests {
    #[test]
    fn includes_the_current_process() {
        let current = std::env::current_exe()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(
            super::running_process_names()
                .unwrap()
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&current))
        );
    }
}
