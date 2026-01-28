//! FFI Exports
//!
//! C ABI function exports for Swift integration.

use std::ffi::c_char;
use std::path::PathBuf;

use vimo_ffi::{cstr_to_str, ffi_boundary, str_to_cstring};

use dev_runner_core::adapter::{
    AdapterRegistry, BuildOptions, DeviceState, DeviceType, RunOptions,
};

use super::types::*;
use crate::get_runtime;

// ============================================================================
// Lifecycle
// ============================================================================

/// Initialize DevRunner
///
/// Returns a handle that must be freed with `dev_runner_free`.
///
/// # Safety
/// Returns null on failure.
#[no_mangle]
pub extern "C" fn dev_runner_init() -> *mut DevRunnerHandle {
    vimo_ffi::ffi_boundary_simple(std::ptr::null_mut(), || {
        let handle = Box::new(DevRunnerHandle::new());
        Box::into_raw(handle)
    })
}

/// Free DevRunner handle
///
/// # Safety
/// `handle` must be a valid pointer returned by `dev_runner_init`, or null.
#[no_mangle]
pub unsafe extern "C" fn dev_runner_free(handle: *mut DevRunnerHandle) {
    if !handle.is_null() {
        drop(Box::from_raw(handle));
    }
}

/// Free a string allocated by DevRunner
///
/// # Safety
/// `ptr` must be a valid pointer returned by DevRunner functions, or null.
#[no_mangle]
pub unsafe extern "C" fn dev_runner_free_string(ptr: *mut c_char) {
    vimo_ffi::vimo_ffi_free_string(ptr);
}

// ============================================================================
// Project Detection
// ============================================================================

/// Detect all projects at path
///
/// Returns JSON array of ProjectInfo. If the path itself is a project,
/// returns just that. Otherwise searches subdirectories (up to 3 levels).
///
/// # Safety
/// - `path` must be valid UTF-8 C string
/// - Returned string must be freed with `dev_runner_free_string`
#[no_mangle]
pub unsafe extern "C" fn dev_runner_detect(
    path: *const c_char,
    out_error: *mut *mut c_char,
) -> *mut c_char {
    ffi_boundary(out_error, std::ptr::null_mut(), || {
        let path_str = cstr_to_str(path).map_err(|e| e.to_string())?;
        let path = PathBuf::from(path_str);

        let adapters = AdapterRegistry::detect(&path);

        let projects: Vec<ProjectInfo> = adapters
            .iter()
            .map(|a| ProjectInfo {
                adapter_type: a.adapter_type().to_string(),
                name: a.name().to_string(),
                path: a.path().to_string_lossy().to_string(),
                bundle_id: a.bundle_id(),
            })
            .collect();

        let json = serde_json::to_string(&projects).map_err(|e| e.to_string())?;
        str_to_cstring(&json).map_err(|e| e.to_string())
    })
}

/// Open a project and add it to the handle
///
/// This opens the project at the exact path given. Use `dev_runner_detect`
/// first to find projects in a directory, then open specific ones by their path.
///
/// Returns the project path (key) on success.
///
/// # Safety
/// - `handle` must be valid
/// - `project_path` must be valid UTF-8 C string (exact project path)
#[no_mangle]
pub unsafe extern "C" fn dev_runner_open(
    handle: *mut DevRunnerHandle,
    project_path: *const c_char,
    out_error: *mut *mut c_char,
) -> *mut c_char {
    ffi_boundary(out_error, std::ptr::null_mut(), || {
        if handle.is_null() {
            return Err("null handle".to_string());
        }

        let path_str = cstr_to_str(project_path).map_err(|e| e.to_string())?;
        let path = PathBuf::from(path_str);

        // Try to detect project at this exact path only (no recursive search)
        let adapter = dev_runner_core::adapter::xcode::XcodeAdapter::detect(&path)
            .or_else(|| dev_runner_core::adapter::node::NodeAdapter::detect(&path))
            .ok_or_else(|| format!("no project at path: {}", path_str))?;

        let handle_ref = &*handle;
        let key = handle_ref.add_project(adapter);

        str_to_cstring(&key).map_err(|e| e.to_string())
    })
}

/// Close a project and remove it from the handle
///
/// # Safety
/// - `handle` must be valid
/// - `project_path` must be valid UTF-8 C string
#[no_mangle]
pub unsafe extern "C" fn dev_runner_close(
    handle: *mut DevRunnerHandle,
    project_path: *const c_char,
    out_error: *mut *mut c_char,
) -> bool {
    ffi_boundary(out_error, false, || {
        if handle.is_null() {
            return Err("null handle".to_string());
        }

        let path_str = cstr_to_str(project_path).map_err(|e| e.to_string())?;
        let handle_ref = &*handle;

        if handle_ref.remove_project(path_str) {
            Ok(true)
        } else {
            Err("project not found".to_string())
        }
    })
}

/// List all opened projects
///
/// Returns JSON array of ProjectInfo.
///
/// # Safety
/// - `handle` must be valid
/// - Returned string must be freed
#[no_mangle]
pub unsafe extern "C" fn dev_runner_list_opened(
    handle: *const DevRunnerHandle,
    out_error: *mut *mut c_char,
) -> *mut c_char {
    ffi_boundary(out_error, std::ptr::null_mut(), || {
        if handle.is_null() {
            return Err("null handle".to_string());
        }

        let handle_ref = &*handle;
        let projects_guard = handle_ref.projects.read();

        let projects: Vec<ProjectInfo> = projects_guard
            .values()
            .map(|a| ProjectInfo {
                adapter_type: a.adapter_type().to_string(),
                name: a.name().to_string(),
                path: a.path().to_string_lossy().to_string(),
                bundle_id: a.bundle_id(),
            })
            .collect();

        let json = serde_json::to_string(&projects).map_err(|e| e.to_string())?;
        str_to_cstring(&json).map_err(|e| e.to_string())
    })
}

/// Scan directory for all projects
///
/// Returns JSON array of ProjectInfo.
///
/// # Safety
/// - `path` must be valid UTF-8 C string
/// - Returned string must be freed with `dev_runner_free_string`
#[no_mangle]
pub unsafe extern "C" fn dev_runner_scan(
    path: *const c_char,
    out_error: *mut *mut c_char,
) -> *mut c_char {
    ffi_boundary(out_error, std::ptr::null_mut(), || {
        let path_str = cstr_to_str(path).map_err(|e| e.to_string())?;
        let path = PathBuf::from(path_str);

        let adapters = AdapterRegistry::scan(&path);

        let projects: Vec<ProjectInfo> = adapters
            .iter()
            .map(|a| ProjectInfo {
                adapter_type: a.adapter_type().to_string(),
                name: a.name().to_string(),
                path: a.path().to_string_lossy().to_string(),
                bundle_id: a.bundle_id(),
            })
            .collect();

        let json = serde_json::to_string(&projects).map_err(|e| e.to_string())?;
        str_to_cstring(&json).map_err(|e| e.to_string())
    })
}

// ============================================================================
// Targets & Devices
// ============================================================================

/// List targets (schemes, scripts) for a project
///
/// Returns JSON array of TargetInfo.
///
/// # Safety
/// - `handle` must be valid
/// - `project_path` must be valid UTF-8 (path of opened project)
/// - Returned string must be freed with `dev_runner_free_string`
#[no_mangle]
pub unsafe extern "C" fn dev_runner_list_targets(
    handle: *const DevRunnerHandle,
    project_path: *const c_char,
    out_error: *mut *mut c_char,
) -> *mut c_char {
    ffi_boundary(out_error, std::ptr::null_mut(), || {
        if handle.is_null() {
            return Err("null handle".to_string());
        }

        let path_str = cstr_to_str(project_path).map_err(|e| e.to_string())?;
        let handle_ref = &*handle;
        let projects_guard = handle_ref.projects.read();
        let adapter = projects_guard
            .get(path_str)
            .ok_or_else(|| format!("project not found: {}", path_str))?;

        let targets: Vec<TargetInfo> = adapter
            .targets()
            .into_iter()
            .map(|t| TargetInfo {
                name: t.name,
                target_type: t.target_type,
                description: t.description,
            })
            .collect();

        let json = serde_json::to_string(&targets).map_err(|e| e.to_string())?;
        str_to_cstring(&json).map_err(|e| e.to_string())
    })
}

/// List available devices for a project
///
/// Returns JSON array of DeviceInfo.
///
/// # Safety
/// - `handle` must be valid
/// - `project_path` must be valid UTF-8 (path of opened project)
/// - Returned string must be freed with `dev_runner_free_string`
#[no_mangle]
pub unsafe extern "C" fn dev_runner_list_devices(
    handle: *const DevRunnerHandle,
    project_path: *const c_char,
    out_error: *mut *mut c_char,
) -> *mut c_char {
    ffi_boundary(out_error, std::ptr::null_mut(), || {
        if handle.is_null() {
            return Err("null handle".to_string());
        }

        let path_str = cstr_to_str(project_path).map_err(|e| e.to_string())?;
        let handle_ref = &*handle;
        let projects_guard = handle_ref.projects.read();
        let adapter = projects_guard
            .get(path_str)
            .ok_or_else(|| format!("project not found: {}", path_str))?;

        let devices: Vec<DeviceInfo> = adapter
            .devices()
            .into_iter()
            .map(|d| DeviceInfo {
                id: d.id,
                name: d.name,
                device_type: match d.device_type {
                    DeviceType::Mac => "mac".to_string(),
                    DeviceType::Simulator => "simulator".to_string(),
                    DeviceType::Physical => "physical".to_string(),
                },
                os_version: d.os_version,
                state: match d.state {
                    DeviceState::Available => "available".to_string(),
                    DeviceState::Unavailable => "unavailable".to_string(),
                    DeviceState::Unknown => "unknown".to_string(),
                },
            })
            .collect();

        let json = serde_json::to_string(&devices).map_err(|e| e.to_string())?;
        str_to_cstring(&json).map_err(|e| e.to_string())
    })
}

// ============================================================================
// Command Generation
// ============================================================================

/// Helper to find device by ID
fn find_device(
    adapter: &dyn dev_runner_core::adapter::RunnerAdapter,
    device_id: &str,
) -> Option<dev_runner_core::adapter::Device> {
    adapter.devices().into_iter().find(|d| d.id == device_id)
}

/// Get build command
///
/// Returns JSON CommandInfo, or null if build not applicable.
///
/// # Safety
/// - `handle` must be valid
/// - `project_path` must be valid UTF-8 (path of opened project)
/// - `target` must be valid UTF-8
/// - `options_json` can be null (uses defaults) or valid JSON
/// - Returned string must be freed
#[no_mangle]
pub unsafe extern "C" fn dev_runner_build_cmd(
    handle: *const DevRunnerHandle,
    project_path: *const c_char,
    target: *const c_char,
    options_json: *const c_char,
    out_error: *mut *mut c_char,
) -> *mut c_char {
    ffi_boundary(out_error, std::ptr::null_mut(), || {
        if handle.is_null() {
            return Err("null handle".to_string());
        }

        let path_str = cstr_to_str(project_path).map_err(|e| e.to_string())?;
        let target_str = cstr_to_str(target).map_err(|e| e.to_string())?;

        // Parse options
        let input: BuildOptionsInput = if options_json.is_null() {
            BuildOptionsInput::default()
        } else {
            let json_str = cstr_to_str(options_json).map_err(|e| e.to_string())?;
            serde_json::from_str(json_str).map_err(|e| e.to_string())?
        };

        let handle_ref = &*handle;
        let projects_guard = handle_ref.projects.read();
        let adapter = projects_guard
            .get(path_str)
            .ok_or_else(|| format!("project not found: {}", path_str))?;

        // Find device if specified
        let device = input
            .device_id
            .as_ref()
            .and_then(|id| find_device(adapter.as_ref(), id));

        let options = BuildOptions {
            config: input.config.unwrap_or_else(|| "Debug".to_string()),
            clean: input.clean,
            device,
            env: input.env,
        };

        let cmd = adapter
            .build_cmd(target_str, &options)
            .ok_or_else(|| "build not supported".to_string())?;

        let info = CommandInfo::from(&cmd);
        let json = serde_json::to_string(&info).map_err(|e| e.to_string())?;
        str_to_cstring(&json).map_err(|e| e.to_string())
    })
}

/// Get install command (Xcode only)
///
/// Returns JSON CommandInfo, or null if install not needed.
///
/// # Safety
/// Same as `dev_runner_build_cmd`
#[no_mangle]
pub unsafe extern "C" fn dev_runner_install_cmd(
    handle: *const DevRunnerHandle,
    project_path: *const c_char,
    target: *const c_char,
    options_json: *const c_char,
    out_error: *mut *mut c_char,
) -> *mut c_char {
    ffi_boundary(out_error, std::ptr::null_mut(), || {
        if handle.is_null() {
            return Err("null handle".to_string());
        }

        let path_str = cstr_to_str(project_path).map_err(|e| e.to_string())?;
        let target_str = cstr_to_str(target).map_err(|e| e.to_string())?;

        let input: RunOptionsInput = if options_json.is_null() {
            RunOptionsInput::default()
        } else {
            let json_str = cstr_to_str(options_json).map_err(|e| e.to_string())?;
            serde_json::from_str(json_str).map_err(|e| e.to_string())?
        };

        let handle_ref = &*handle;
        let projects_guard = handle_ref.projects.read();
        let adapter = projects_guard
            .get(path_str)
            .ok_or_else(|| format!("project not found: {}", path_str))?;

        let device = input
            .device_id
            .as_ref()
            .and_then(|id| find_device(adapter.as_ref(), id));

        let options = RunOptions {
            device,
            env: input.env,
            args: input.args,
        };

        let cmd = adapter
            .install_cmd(target_str, &options)
            .ok_or_else(|| "install not needed".to_string())?;

        let info = CommandInfo::from(&cmd);
        let json = serde_json::to_string(&info).map_err(|e| e.to_string())?;
        str_to_cstring(&json).map_err(|e| e.to_string())
    })
}

/// Get run command
///
/// Returns JSON CommandInfo.
///
/// # Safety
/// Same as `dev_runner_build_cmd`
#[no_mangle]
pub unsafe extern "C" fn dev_runner_run_cmd(
    handle: *const DevRunnerHandle,
    project_path: *const c_char,
    target: *const c_char,
    options_json: *const c_char,
    out_error: *mut *mut c_char,
) -> *mut c_char {
    ffi_boundary(out_error, std::ptr::null_mut(), || {
        if handle.is_null() {
            return Err("null handle".to_string());
        }

        let path_str = cstr_to_str(project_path).map_err(|e| e.to_string())?;
        let target_str = cstr_to_str(target).map_err(|e| e.to_string())?;

        let input: RunOptionsInput = if options_json.is_null() {
            RunOptionsInput::default()
        } else {
            let json_str = cstr_to_str(options_json).map_err(|e| e.to_string())?;
            serde_json::from_str(json_str).map_err(|e| e.to_string())?
        };

        let handle_ref = &*handle;
        let projects_guard = handle_ref.projects.read();
        let adapter = projects_guard
            .get(path_str)
            .ok_or_else(|| format!("project not found: {}", path_str))?;

        let device = input
            .device_id
            .as_ref()
            .and_then(|id| find_device(adapter.as_ref(), id));

        let options = RunOptions {
            device,
            env: input.env,
            args: input.args,
        };

        let cmd = adapter.run_cmd(target_str, &options);

        let info = CommandInfo::from(&cmd);
        let json = serde_json::to_string(&info).map_err(|e| e.to_string())?;
        str_to_cstring(&json).map_err(|e| e.to_string())
    })
}

/// Get log command
///
/// Returns JSON CommandInfo, or null if not applicable.
///
/// # Safety
/// Same as `dev_runner_build_cmd`
#[no_mangle]
pub unsafe extern "C" fn dev_runner_log_cmd(
    handle: *const DevRunnerHandle,
    project_path: *const c_char,
    target: *const c_char,
    options_json: *const c_char,
    out_error: *mut *mut c_char,
) -> *mut c_char {
    ffi_boundary(out_error, std::ptr::null_mut(), || {
        if handle.is_null() {
            return Err("null handle".to_string());
        }

        let path_str = cstr_to_str(project_path).map_err(|e| e.to_string())?;
        let target_str = cstr_to_str(target).map_err(|e| e.to_string())?;

        let input: RunOptionsInput = if options_json.is_null() {
            RunOptionsInput::default()
        } else {
            let json_str = cstr_to_str(options_json).map_err(|e| e.to_string())?;
            serde_json::from_str(json_str).map_err(|e| e.to_string())?
        };

        let handle_ref = &*handle;
        let projects_guard = handle_ref.projects.read();
        let adapter = projects_guard
            .get(path_str)
            .ok_or_else(|| format!("project not found: {}", path_str))?;

        let device = input
            .device_id
            .as_ref()
            .and_then(|id| find_device(adapter.as_ref(), id));

        let options = RunOptions {
            device,
            env: input.env,
            args: input.args,
        };

        let cmd = adapter
            .log_cmd(target_str, &options)
            .ok_or_else(|| "log not supported".to_string())?;

        let info = CommandInfo::from(&cmd);
        let json = serde_json::to_string(&info).map_err(|e| e.to_string())?;
        str_to_cstring(&json).map_err(|e| e.to_string())
    })
}

// ============================================================================
// Process Management
// ============================================================================

/// Start a process
///
/// Returns JSON ProcessStartResult with process_id.
///
/// # Safety
/// - `handle` must be valid
/// - `project_path` must be valid UTF-8 (path of opened project)
/// - `target` must be valid UTF-8 (target name for tracking)
/// - `command_json` must be valid JSON (CommandInfo format)
/// - Returned string must be freed
#[no_mangle]
pub unsafe extern "C" fn dev_runner_start_process(
    handle: *mut DevRunnerHandle,
    project_path: *const c_char,
    target: *const c_char,
    command_json: *const c_char,
    out_error: *mut *mut c_char,
) -> *mut c_char {
    ffi_boundary(out_error, std::ptr::null_mut(), || {
        if handle.is_null() {
            return Err("null handle".to_string());
        }

        let path_str = cstr_to_str(project_path).map_err(|e| e.to_string())?;
        let target_str = cstr_to_str(target).map_err(|e| e.to_string())?;
        let json_str = cstr_to_str(command_json).map_err(|e| e.to_string())?;
        let cmd_info: CommandInfo = serde_json::from_str(json_str).map_err(|e| e.to_string())?;

        // Convert to Command
        let mut cmd = dev_runner_core::adapter::Command::new(&cmd_info.program);
        cmd = cmd.args(cmd_info.args);
        if let Some(cwd) = cmd_info.cwd {
            cmd = cmd.cwd(cwd);
        }
        for (k, v) in cmd_info.env {
            cmd = cmd.env(k, v);
        }

        let handle_ref = &*handle;

        // Get adapter info from projects
        let (adapter_type, project_path_buf) = {
            let projects_guard = handle_ref.projects.read();
            match projects_guard.get(path_str) {
                Some(a) => (a.adapter_type().to_string(), a.path().to_path_buf()),
                None => ("unknown".to_string(), PathBuf::from(path_str)),
            }
        };

        // Start process using runtime
        let rt = get_runtime();
        let result = rt.block_on(async {
            handle_ref
                .process_manager
                .start(cmd, project_path_buf, &adapter_type, target_str)
                .await
        });

        let (process_id, _rx) = result.map_err(|e| e.to_string())?;

        let result = ProcessStartResult { process_id };
        let json = serde_json::to_string(&result).map_err(|e| e.to_string())?;
        str_to_cstring(&json).map_err(|e| e.to_string())
    })
}

/// Stop a process
///
/// # Safety
/// - `handle` must be valid
/// - `process_id` must be valid UTF-8
#[no_mangle]
pub unsafe extern "C" fn dev_runner_stop_process(
    handle: *mut DevRunnerHandle,
    process_id: *const c_char,
    out_error: *mut *mut c_char,
) -> bool {
    ffi_boundary(out_error, false, || {
        if handle.is_null() {
            return Err("null handle".to_string());
        }

        let process_id_str = cstr_to_str(process_id).map_err(|e| e.to_string())?;

        let handle_ref = &*handle;

        let rt = get_runtime();
        rt.block_on(async { handle_ref.process_manager.stop(process_id_str).await })
            .map_err(|e| e.to_string())?;

        Ok(true)
    })
}

/// List all processes
///
/// Returns JSON array of ProcessInfo.
///
/// # Safety
/// - `handle` must be valid
/// - Returned string must be freed
#[no_mangle]
pub unsafe extern "C" fn dev_runner_list_processes(
    handle: *const DevRunnerHandle,
    out_error: *mut *mut c_char,
) -> *mut c_char {
    ffi_boundary(out_error, std::ptr::null_mut(), || {
        if handle.is_null() {
            return Err("null handle".to_string());
        }

        let handle_ref = &*handle;
        let processes = handle_ref.process_manager.list_all();

        let infos: Vec<ProcessInfo> = processes
            .into_iter()
            .map(|p| {
                let (status, error_message) = match &p.status {
                    crate::process::ProcessStatus::Running => ("running".to_string(), None),
                    crate::process::ProcessStatus::Stopped => ("stopped".to_string(), None),
                    crate::process::ProcessStatus::Failed { code, message } => {
                        ("failed".to_string(), Some(format!("[{}] {}", code, message)))
                    }
                };

                ProcessInfo {
                    id: p.id,
                    project_path: p.project_path.to_string_lossy().to_string(),
                    adapter_type: p.adapter_type,
                    target: p.target,
                    pid: p.pid,
                    status,
                    error_message,
                    started_at: p.started_at.timestamp(),
                    ended_at: p.ended_at.map(|t| t.timestamp()),
                }
            })
            .collect();

        let json = serde_json::to_string(&infos).map_err(|e| e.to_string())?;
        str_to_cstring(&json).map_err(|e| e.to_string())
    })
}

/// Get process info by ID
///
/// Returns JSON ProcessInfo, or null if not found.
///
/// # Safety
/// - `handle` must be valid
/// - `process_id` must be valid UTF-8
/// - Returned string must be freed
#[no_mangle]
pub unsafe extern "C" fn dev_runner_get_process(
    handle: *const DevRunnerHandle,
    process_id: *const c_char,
    out_error: *mut *mut c_char,
) -> *mut c_char {
    ffi_boundary(out_error, std::ptr::null_mut(), || {
        if handle.is_null() {
            return Err("null handle".to_string());
        }

        let process_id_str = cstr_to_str(process_id).map_err(|e| e.to_string())?;

        let handle_ref = &*handle;
        let process = handle_ref
            .process_manager
            .get(process_id_str)
            .ok_or_else(|| "process not found".to_string())?;

        let (status, error_message) = match &process.status {
            crate::process::ProcessStatus::Running => ("running".to_string(), None),
            crate::process::ProcessStatus::Stopped => ("stopped".to_string(), None),
            crate::process::ProcessStatus::Failed { code, message } => {
                ("failed".to_string(), Some(format!("[{}] {}", code, message)))
            }
        };

        let info = ProcessInfo {
            id: process.id,
            project_path: process.project_path.to_string_lossy().to_string(),
            adapter_type: process.adapter_type,
            target: process.target,
            pid: process.pid,
            status,
            error_message,
            started_at: process.started_at.timestamp(),
            ended_at: process.ended_at.map(|t| t.timestamp()),
        };

        let json = serde_json::to_string(&info).map_err(|e| e.to_string())?;
        str_to_cstring(&json).map_err(|e| e.to_string())
    })
}

// ============================================================================
// Metrics
// ============================================================================

/// Get process metrics (CPU/memory)
///
/// Returns JSON MetricsInfo, or null if process not found.
///
/// # Safety
/// - `handle` must be valid
/// - `pid` is the system PID (not UUID)
/// - Returned string must be freed
#[no_mangle]
pub unsafe extern "C" fn dev_runner_get_metrics(
    handle: *mut DevRunnerHandle,
    pid: u32,
    out_error: *mut *mut c_char,
) -> *mut c_char {
    ffi_boundary(out_error, std::ptr::null_mut(), || {
        if handle.is_null() {
            return Err("null handle".to_string());
        }

        let handle_ref = &*handle;
        let mut monitor = handle_ref.process_monitor.write();

        let metrics = monitor
            .get_metrics(pid)
            .ok_or_else(|| "process not found".to_string())?;

        let info = MetricsInfo {
            pid: metrics.pid,
            cpu_percent: metrics.cpu_percent,
            memory_bytes: metrics.memory_bytes,
            timestamp: metrics.timestamp.timestamp(),
        };

        let json = serde_json::to_string(&info).map_err(|e| e.to_string())?;
        str_to_cstring(&json).map_err(|e| e.to_string())
    })
}

/// Read process output
///
/// Returns JSON OutputReadResult with lines and next_line number.
///
/// # Safety
/// - `handle` must be valid
/// - `process_id` must be valid UTF-8
/// - `since_line` is the line number to read from (0 = from start)
/// - Returned string must be freed
#[no_mangle]
pub unsafe extern "C" fn dev_runner_read_output(
    handle: *const DevRunnerHandle,
    process_id: *const c_char,
    since_line: u64,
    out_error: *mut *mut c_char,
) -> *mut c_char {
    ffi_boundary(out_error, std::ptr::null_mut(), || {
        if handle.is_null() {
            return Err("null handle".to_string());
        }

        let process_id_str = cstr_to_str(process_id).map_err(|e| e.to_string())?;

        let handle_ref = &*handle;
        let (lines, next_line) = handle_ref.process_manager.read_output(process_id_str, since_line as usize);

        let result = OutputReadResult { lines, next_line };
        let json = serde_json::to_string(&result).map_err(|e| e.to_string())?;
        str_to_cstring(&json).map_err(|e| e.to_string())
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn test_init_and_free() {
        let handle = dev_runner_init();
        assert!(!handle.is_null());
        unsafe { dev_runner_free(handle) };
    }

    #[test]
    fn test_free_null_handle() {
        unsafe { dev_runner_free(std::ptr::null_mut()) };
    }

    #[test]
    fn test_scan_empty_dir() {
        let handle = dev_runner_init();
        let temp_dir = tempfile::tempdir().unwrap();
        let path = CString::new(temp_dir.path().to_str().unwrap()).unwrap();
        let mut error: *mut c_char = std::ptr::null_mut();

        let result = unsafe { dev_runner_scan(path.as_ptr(), &mut error) };

        assert!(!result.is_null());
        let json = unsafe { CString::from_raw(result) };
        let projects: Vec<ProjectInfo> = serde_json::from_str(json.to_str().unwrap()).unwrap();
        assert!(projects.is_empty());

        unsafe { dev_runner_free(handle) };
    }

    #[test]
    fn test_detect_empty_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = CString::new(temp_dir.path().to_str().unwrap()).unwrap();
        let mut error: *mut c_char = std::ptr::null_mut();

        let result = unsafe { dev_runner_detect(path.as_ptr(), &mut error) };

        // Should return empty array, not null
        assert!(!result.is_null());
        let json = unsafe { CString::from_raw(result) };
        let projects: Vec<ProjectInfo> = serde_json::from_str(json.to_str().unwrap()).unwrap();
        assert!(projects.is_empty());
    }

    #[test]
    fn test_open_no_project() {
        let handle = dev_runner_init();
        let temp_dir = tempfile::tempdir().unwrap();
        let path = CString::new(temp_dir.path().to_str().unwrap()).unwrap();
        let mut error: *mut c_char = std::ptr::null_mut();

        let result = unsafe { dev_runner_open(handle, path.as_ptr(), &mut error) };

        // Should fail - no project at path
        assert!(result.is_null());
        assert!(!error.is_null());

        unsafe {
            dev_runner_free_string(error);
            dev_runner_free(handle);
        };
    }

    #[test]
    fn test_list_targets_no_project() {
        let handle = dev_runner_init();
        let fake_path = CString::new("/nonexistent/path").unwrap();
        let mut error: *mut c_char = std::ptr::null_mut();

        let result = unsafe { dev_runner_list_targets(handle, fake_path.as_ptr(), &mut error) };

        // Should fail - project not found
        assert!(result.is_null());
        assert!(!error.is_null());

        unsafe {
            dev_runner_free_string(error);
            dev_runner_free(handle);
        };
    }
}
