use domain::{
    AdminOperation, AdminRejectReason, AdminRequest, PROTOCOL_VERSION, is_forbidden_path,
    normalize_path,
};

pub fn validate_request(request: &AdminRequest) -> Result<(), AdminRejectReason> {
    if request.protocol_version != PROTOCOL_VERSION || request.request_id.0 == 0 {
        return Err(AdminRejectReason::InvalidRequest);
    }
    match &request.operation {
        AdminOperation::CleanSystemTemp { plan_id } if plan_id.0 != 0 => Ok(()),
        AdminOperation::CleanSystemTemp { .. } => Err(AdminRejectReason::InvalidRequest),
        AdminOperation::QueryRecycleBin {
            user_scope: domain::UserScope::CurrentUser,
        } => Ok(()),
        AdminOperation::QueryRecycleBin { .. } => Err(AdminRejectReason::UnsupportedScope),
        AdminOperation::MoveToRecycleBin { path } => {
            let normalized =
                normalize_path(path.display()).map_err(|_| AdminRejectReason::InvalidRequest)?;
            if normalized.display() != path.display()
                || normalized.extended() != path.extended()
                || is_forbidden_path(&normalized)
            {
                return Err(AdminRejectReason::ForbiddenPath);
            }
            Ok(())
        }
        AdminOperation::DeleteFile { path, explicit, .. } => {
            let normalized =
                normalize_path(path.display()).map_err(|_| AdminRejectReason::InvalidRequest)?;
            if normalized.display() != path.display()
                || normalized.extended() != path.extended()
                || (is_forbidden_path(&normalized)
                    && !(*explicit && domain::is_explicit_cleanup_allowed(&normalized)))
            {
                return Err(AdminRejectReason::ForbiddenPath);
            }
            Ok(())
        }
        AdminOperation::DisableHibernation | AdminOperation::DisablePagingFiles => Ok(()),
        AdminOperation::AnalyzeComponentStore => Ok(()),
        AdminOperation::CleanComponentStore { plan_id } if plan_id.0 != 0 => Ok(()),
        AdminOperation::CleanComponentStore { .. } => Err(AdminRejectReason::InvalidRequest),
    }
}

#[cfg(windows)]
mod windows_acl {
    use std::{mem, ptr, slice};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, HANDLE, LocalFree},
        Security::Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        },
        Security::{
            GetLengthSid, GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
            TokenUser,
        },
        System::Pipes::GetNamedPipeClientProcessId,
        System::Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    };

    pub struct PipeSecurity {
        descriptor: *mut core::ffi::c_void,
        pub attributes: SECURITY_ATTRIBUTES,
        sid: Vec<u8>,
    }

    impl PipeSecurity {
        pub fn current_user() -> Result<Self, String> {
            let mut token = ptr::null_mut();
            if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
                return Err(format!("cannot open helper token: {}", unsafe {
                    GetLastError()
                }));
            }
            let sid = token_sid(token);
            unsafe {
                CloseHandle(token);
            }
            let sid = sid?;
            let sid_string = sid_string(&sid)?;
            let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid_string})")
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let mut descriptor = ptr::null_mut();
            let mut size = 0;
            if unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    1,
                    &mut descriptor,
                    &mut size,
                )
            } == 0
            {
                return Err(format!("security descriptor: {}", unsafe {
                    GetLastError()
                }));
            }
            let attributes = SECURITY_ATTRIBUTES {
                nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            };
            Ok(Self {
                descriptor,
                attributes,
                sid,
            })
        }

        pub(crate) fn validate_client(&self, pipe: HANDLE) -> Result<(), String> {
            let mut pid = 0;
            if unsafe { GetNamedPipeClientProcessId(pipe, &mut pid) } == 0 {
                return Err("cannot identify named-pipe client".into());
            }
            let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
            if process.is_null() {
                return Err("cannot open named-pipe client".into());
            }
            let mut token = std::ptr::null_mut();
            let opened = unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } != 0;
            let result = if opened {
                token_sid(token).map(|sid| sid == self.sid)
            } else {
                Err("cannot open client token".into())
            };
            if !token.is_null() {
                unsafe {
                    CloseHandle(token);
                }
            }
            unsafe {
                CloseHandle(process);
            }
            match result {
                Ok(true) => Ok(()),
                Ok(false) => Err("named-pipe client is not the current user".into()),
                Err(error) => Err(error),
            }
        }
    }

    impl Drop for PipeSecurity {
        fn drop(&mut self) {
            if !self.descriptor.is_null() {
                unsafe {
                    LocalFree(self.descriptor);
                }
            }
        }
    }

    fn token_sid(token: HANDLE) -> Result<Vec<u8>, String> {
        let mut required = 0;
        unsafe {
            GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut required);
        }
        if required == 0 {
            return Err(format!("token size: {}", unsafe { GetLastError() }));
        }
        let mut buffer = vec![0u8; required as usize];
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                required,
                &mut required,
            )
        } == 0
        {
            return Err(format!("token user: {}", unsafe { GetLastError() }));
        }
        let user = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };
        let length = unsafe { GetLengthSid(user.User.Sid) };
        if length == 0 {
            return Err("invalid token SID".into());
        }
        Ok(unsafe { slice::from_raw_parts(user.User.Sid.cast::<u8>(), length as usize) }.to_vec())
    }

    fn sid_string(sid: &[u8]) -> Result<String, String> {
        let mut pointer = ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(sid.as_ptr().cast_mut().cast(), &mut pointer) } == 0 {
            return Err(format!("SID conversion: {}", unsafe { GetLastError() }));
        }
        let mut length = 0;
        while unsafe { *pointer.add(length) } != 0 {
            length += 1;
        }
        let result = String::from_utf16_lossy(unsafe { slice::from_raw_parts(pointer, length) });
        unsafe {
            LocalFree(pointer.cast());
        }
        Ok(result)
    }
}

#[cfg(windows)]
pub use windows_acl::PipeSecurity;

#[cfg(test)]
mod tests {
    use super::validate_request;
    use domain::{AdminOperation, AdminRequest, PROTOCOL_VERSION, PlanId, RequestId, UserScope};

    #[test]
    fn pipe_security_can_read_the_current_process_token() {
        super::PipeSecurity::current_user()
            .expect("the IPC ACL must use a token handle, not a process handle");
    }

    #[test]
    fn permanent_delete_rejects_core_paths_even_with_explicit_selection() {
        for explicit in [false, true] {
            let request = AdminRequest {
                protocol_version: PROTOCOL_VERSION,
                request_id: RequestId(3),
                operation: AdminOperation::DeleteFile {
                    path: domain::normalize_path(r"C:\Windows\System32\protected.dll").unwrap(),
                    identity: domain::FileIdentity {
                        volume_serial: 1,
                        file_index: 2,
                    },
                    size: 1,
                    last_write_time: 1,
                    explicit,
                },
            };
            assert_eq!(
                validate_request(&request),
                Err(domain::AdminRejectReason::ForbiddenPath)
            );
        }
        let request = AdminRequest {
            protocol_version: PROTOCOL_VERSION,
            request_id: RequestId(4),
            operation: AdminOperation::DeleteFile {
                path: domain::normalize_path(r"C:\Windows\Temp\disposable.tmp").unwrap(),
                identity: domain::FileIdentity {
                    volume_serial: 1,
                    file_index: 2,
                },
                size: 1,
                last_write_time: 1,
                explicit: false,
            },
        };
        assert_eq!(validate_request(&request), Ok(()));
    }

    #[test]
    fn permanent_delete_accepts_rebuildable_system_garbage() {
        for path in [
            r"C:\Windows\Logs\CBS\CBS.log",
            r"C:\Windows\Minidump\mini.dmp",
            r"C:\Windows\SoftwareDistribution\Download\update.cab",
            r"C:\ProgramData\Microsoft\Windows\DeliveryOptimization\Cache\chunk.bin",
        ] {
            let request = AdminRequest {
                protocol_version: PROTOCOL_VERSION,
                request_id: RequestId(5),
                operation: AdminOperation::DeleteFile {
                    path: domain::normalize_path(path).unwrap(),
                    identity: domain::FileIdentity {
                        volume_serial: 1,
                        file_index: 2,
                    },
                    size: 4096,
                    last_write_time: 1,
                    explicit: false,
                },
            };
            assert_eq!(validate_request(&request), Ok(()), "{path}");
        }
    }

    #[test]
    fn rejects_unsupported_scope_and_zero_plan() {
        assert!(
            validate_request(&AdminRequest {
                protocol_version: PROTOCOL_VERSION,
                request_id: RequestId(1),
                operation: AdminOperation::CleanSystemTemp { plan_id: PlanId(0) }
            })
            .is_err()
        );
        assert!(
            validate_request(&AdminRequest {
                protocol_version: PROTOCOL_VERSION,
                request_id: RequestId(2),
                operation: AdminOperation::QueryRecycleBin {
                    user_scope: UserScope::AllUsers
                }
            })
            .is_err()
        );
    }

    #[test]
    fn accepts_only_the_fixed_system_storage_operations() {
        for operation in [
            AdminOperation::DisableHibernation,
            AdminOperation::DisablePagingFiles,
        ] {
            assert_eq!(
                validate_request(&AdminRequest {
                    protocol_version: PROTOCOL_VERSION,
                    request_id: RequestId(6),
                    operation,
                }),
                Ok(())
            );
        }
    }
}
