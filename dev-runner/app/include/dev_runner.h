#include <cstdarg>
#include <cstdint>
#include <cstdlib>
#include <ostream>
#include <new>

/// DevRunner handle - opaque pointer for Swift
struct DevRunnerHandle;

extern "C" {

/// Initialize DevRunner
///
/// Returns a handle that must be freed with `dev_runner_free`.
///
/// # Safety
/// Returns null on failure.
DevRunnerHandle *dev_runner_init();

/// Free DevRunner handle
///
/// # Safety
/// `handle` must be a valid pointer returned by `dev_runner_init`, or null.
void dev_runner_free(DevRunnerHandle *handle);

/// Free a string allocated by DevRunner
///
/// # Safety
/// `ptr` must be a valid pointer returned by DevRunner functions, or null.
void dev_runner_free_string(char *ptr);

/// Detect all projects at path
///
/// Returns JSON array of ProjectInfo. If the path itself is a project,
/// returns just that. Otherwise searches subdirectories (up to 3 levels).
///
/// # Safety
/// - `path` must be valid UTF-8 C string
/// - Returned string must be freed with `dev_runner_free_string`
char *dev_runner_detect(const char *path, char **out_error);

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
char *dev_runner_open(DevRunnerHandle *handle, const char *project_path, char **out_error);

/// Close a project and remove it from the handle
///
/// # Safety
/// - `handle` must be valid
/// - `project_path` must be valid UTF-8 C string
bool dev_runner_close(DevRunnerHandle *handle, const char *project_path, char **out_error);

/// List all opened projects
///
/// Returns JSON array of ProjectInfo.
///
/// # Safety
/// - `handle` must be valid
/// - Returned string must be freed
char *dev_runner_list_opened(const DevRunnerHandle *handle, char **out_error);

/// Scan directory for all projects
///
/// Returns JSON array of ProjectInfo.
///
/// # Safety
/// - `path` must be valid UTF-8 C string
/// - Returned string must be freed with `dev_runner_free_string`
char *dev_runner_scan(const char *path, char **out_error);

/// List targets (schemes, scripts) for a project
///
/// Returns JSON array of TargetInfo.
///
/// # Safety
/// - `handle` must be valid
/// - `project_path` must be valid UTF-8 (path of opened project)
/// - Returned string must be freed with `dev_runner_free_string`
char *dev_runner_list_targets(const DevRunnerHandle *handle,
                              const char *project_path,
                              char **out_error);

/// List available devices for a project
///
/// Returns JSON array of DeviceInfo.
///
/// # Safety
/// - `handle` must be valid
/// - `project_path` must be valid UTF-8 (path of opened project)
/// - Returned string must be freed with `dev_runner_free_string`
char *dev_runner_list_devices(const DevRunnerHandle *handle,
                              const char *project_path,
                              char **out_error);

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
char *dev_runner_build_cmd(const DevRunnerHandle *handle,
                           const char *project_path,
                           const char *target,
                           const char *options_json,
                           char **out_error);

/// Get install command (Xcode only)
///
/// Returns JSON CommandInfo, or null if install not needed.
///
/// # Safety
/// Same as `dev_runner_build_cmd`
char *dev_runner_install_cmd(const DevRunnerHandle *handle,
                             const char *project_path,
                             const char *target,
                             const char *options_json,
                             char **out_error);

/// Get run command
///
/// Returns JSON CommandInfo.
///
/// # Safety
/// Same as `dev_runner_build_cmd`
char *dev_runner_run_cmd(const DevRunnerHandle *handle,
                         const char *project_path,
                         const char *target,
                         const char *options_json,
                         char **out_error);

/// Get log command
///
/// Returns JSON CommandInfo, or null if not applicable.
///
/// # Safety
/// Same as `dev_runner_build_cmd`
char *dev_runner_log_cmd(const DevRunnerHandle *handle,
                         const char *project_path,
                         const char *target,
                         const char *options_json,
                         char **out_error);

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
char *dev_runner_start_process(DevRunnerHandle *handle,
                               const char *project_path,
                               const char *target,
                               const char *command_json,
                               char **out_error);

/// Stop a process
///
/// # Safety
/// - `handle` must be valid
/// - `process_id` must be valid UTF-8
bool dev_runner_stop_process(DevRunnerHandle *handle, const char *process_id, char **out_error);

/// List all processes
///
/// Returns JSON array of ProcessInfo.
///
/// # Safety
/// - `handle` must be valid
/// - Returned string must be freed
char *dev_runner_list_processes(const DevRunnerHandle *handle, char **out_error);

/// Get process info by ID
///
/// Returns JSON ProcessInfo, or null if not found.
///
/// # Safety
/// - `handle` must be valid
/// - `process_id` must be valid UTF-8
/// - Returned string must be freed
char *dev_runner_get_process(const DevRunnerHandle *handle,
                             const char *process_id,
                             char **out_error);

/// Get process metrics (CPU/memory)
///
/// Returns JSON MetricsInfo, or null if process not found.
///
/// # Safety
/// - `handle` must be valid
/// - `pid` is the system PID (not UUID)
/// - Returned string must be freed
char *dev_runner_get_metrics(DevRunnerHandle *handle, uint32_t pid, char **out_error);

/// Read process output
///
/// Returns JSON OutputReadResult with lines and next_line number.
///
/// # Safety
/// - `handle` must be valid
/// - `process_id` must be valid UTF-8
/// - `since_line` is the line number to read from (0 = from start)
/// - Returned string must be freed
char *dev_runner_read_output(const DevRunnerHandle *handle,
                             const char *process_id,
                             uint64_t since_line,
                             char **out_error);

}  // extern "C"
