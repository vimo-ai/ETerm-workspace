//
//  BridgingHeader.h
//  DevRunner
//
//  Imports C FFI functions from dev-runner-app Rust library
//  and Sugarloaf terminal FFI
//

#ifndef BridgingHeader_h
#define BridgingHeader_h

#include <stdint.h>
#include <stdbool.h>

// Sugarloaf Terminal FFI
#include "../Terminal/Libs/SugarloafBridge.h"

// Opaque handle
typedef struct DevRunnerHandle DevRunnerHandle;

// Lifecycle
DevRunnerHandle* dev_runner_init(void);
void dev_runner_free(DevRunnerHandle* handle);
void dev_runner_free_string(char* ptr);

// Project detection
char* dev_runner_detect(const char* path, char** out_error);
char* dev_runner_scan(const char* path, char** out_error);

// Multi-project management
char* dev_runner_open(DevRunnerHandle* handle, const char* project_path, char** out_error);
bool dev_runner_close(DevRunnerHandle* handle, const char* project_path, char** out_error);
char* dev_runner_list_opened(const DevRunnerHandle* handle, char** out_error);

// Targets & Devices (now require project_path)
char* dev_runner_list_targets(const DevRunnerHandle* handle, const char* project_path, char** out_error);
char* dev_runner_list_devices(const DevRunnerHandle* handle, const char* project_path, char** out_error);

// Command generation (now require project_path)
char* dev_runner_build_cmd(const DevRunnerHandle* handle, const char* project_path, const char* target, const char* options_json, char** out_error);
char* dev_runner_install_cmd(const DevRunnerHandle* handle, const char* project_path, const char* target, const char* options_json, char** out_error);
char* dev_runner_run_cmd(const DevRunnerHandle* handle, const char* project_path, const char* target, const char* options_json, char** out_error);
char* dev_runner_log_cmd(const DevRunnerHandle* handle, const char* project_path, const char* target, const char* options_json, char** out_error);

// Process management (now require project_path and target)
char* dev_runner_start_process(DevRunnerHandle* handle, const char* project_path, const char* target, const char* command_json, char** out_error);
bool dev_runner_stop_process(DevRunnerHandle* handle, const char* process_id, char** out_error);
char* dev_runner_list_processes(const DevRunnerHandle* handle, char** out_error);
char* dev_runner_get_process(const DevRunnerHandle* handle, const char* process_id, char** out_error);

// Metrics
char* dev_runner_get_metrics(DevRunnerHandle* handle, uint32_t pid, char** out_error);

// Output
char* dev_runner_read_output(const DevRunnerHandle* handle, const char* process_id, uint64_t since_line, char** out_error);

#endif /* BridgingHeader_h */
