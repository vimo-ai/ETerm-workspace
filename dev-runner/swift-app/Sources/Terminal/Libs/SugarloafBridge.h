//
//  SugarloafBridge.h
//  ETerm
//
//  Created by 💻higuaifan on 2025/11/16.
//

#ifndef SugarloafBridge_h
#define SugarloafBridge_h

#include <stddef.h>
#include <stdbool.h>
#include <stdint.h>

typedef struct {
    float cell_width;
    float cell_height;
    float line_height;
} SugarloafFontMetrics;

// =============================================================================
// TerminalPool API (New Architecture - Multi-terminal + Unified Render)
// =============================================================================

/// TerminalPool handle (opaque pointer)
typedef void* TerminalPoolHandle;

/// Free string returned from Rust
void rio_free_string(char* s);

/// App configuration for TerminalPool
typedef struct {
    uint16_t cols;
    uint16_t rows;
    float font_size;
    float line_height;
    float scale;
    void* window_handle;
    void* display_handle;
    float window_width;
    float window_height;
    uint32_t history_size;
    /// Log buffer size (number of lines to retain)
    /// - 0 = disabled (default, for ETerm)
    /// - >0 = enabled (for dev-runner log capture)
    uint32_t log_buffer_size;
} TerminalPoolConfig;

/// Terminal event types
typedef enum {
    TerminalEventType_Wakeup = 0,
    TerminalEventType_Render = 1,
    TerminalEventType_CursorBlink = 2,
    TerminalEventType_Bell = 3,
    TerminalEventType_TitleChanged = 4,
    TerminalEventType_Damaged = 5,
    TerminalEventType_CurrentDirectoryChanged = 6,
    TerminalEventType_CommandExecuted = 7,
} TerminalPoolEventType;

/// Terminal event
typedef struct {
    TerminalPoolEventType event_type;
    uint64_t data;  // terminal_id for multi-terminal events
} TerminalPoolEvent;

/// Event callback type
typedef void (*TerminalPoolEventCallback)(void* context, TerminalPoolEvent event);

/// String event callback type (for events with string data)
///
/// Used for events that carry string payloads:
/// - CurrentDirectoryChanged: data contains the new directory path
/// - CommandExecuted: data contains the executed command
///
/// @param context User context pointer
/// @param event_type Event type
/// @param terminal_id Terminal ID
/// @param data UTF-8 encoded string data (null-terminated)
typedef void (*TerminalPoolStringEventCallback)(
    void* context,
    TerminalPoolEventType event_type,
    size_t terminal_id,
    const char* data
);

/// Create TerminalPool
///
/// Returns: Handle on success, NULL on failure
TerminalPoolHandle terminal_pool_create(TerminalPoolConfig config);

/// Destroy TerminalPool
void terminal_pool_destroy(TerminalPoolHandle handle);

/// Create new terminal
///
/// Returns: Terminal ID (>= 1) on success, -1 on failure
int32_t terminal_pool_create_terminal(
    TerminalPoolHandle handle,
    uint16_t cols,
    uint16_t rows
);

/// Create new terminal with working directory
///
/// Returns: Terminal ID (>= 1) on success, -1 on failure
int32_t terminal_pool_create_terminal_with_cwd(
    TerminalPoolHandle handle,
    uint16_t cols,
    uint16_t rows,
    const char* working_dir
);

/// Create new terminal with specific ID (for session restore)
///
/// Returns: Terminal ID on success, -1 on failure
int64_t terminal_pool_create_terminal_with_id(
    TerminalPoolHandle handle,
    int64_t id,
    uint16_t cols,
    uint16_t rows
);

/// Create new terminal with specific ID and working directory (for session restore)
///
/// Returns: Terminal ID on success, -1 on failure
int64_t terminal_pool_create_terminal_with_id_and_cwd(
    TerminalPoolHandle handle,
    int64_t id,
    uint16_t cols,
    uint16_t rows,
    const char* working_dir
);

/// Close terminal
bool terminal_pool_close_terminal(
    TerminalPoolHandle handle,
    size_t terminal_id
);

/// Get terminal's current working directory (via proc_pidinfo)
///
/// Note: This returns the foreground process's CWD. If a child process is running
/// (like vim, claude), it may return the child's CWD instead of the shell's.
/// Prefer using `terminal_pool_get_cached_cwd` for OSC 7 cached CWD.
///
/// Returns a string that must be freed with rio_free_string
char* terminal_pool_get_cwd(
    TerminalPoolHandle handle,
    size_t terminal_id
);

/// Get terminal's cached working directory (via OSC 7)
///
/// Shell reports CWD via OSC 7 escape sequence. This is more reliable than get_cwd:
/// - Not affected by child processes (like vim, claude)
/// - Shell knows its own directory best
/// - Updated immediately after each cd
///
/// Returns NULL if OSC 7 cache is empty (shell not configured or just started).
/// Returns a string that must be freed with rio_free_string
char* terminal_pool_get_cached_cwd(
    TerminalPoolHandle handle,
    size_t terminal_id
);

/// Get terminal's foreground process name
///
/// Returns the name of the current foreground process (e.g., "vim", "cargo", "python")
/// If the foreground process is the shell itself, returns the shell name (e.g., "zsh", "bash")
///
/// Returns a string that must be freed with rio_free_string
char* terminal_pool_get_foreground_process_name(
    TerminalPoolHandle handle,
    size_t terminal_id
);

/// Check if terminal has a running child process (non-shell)
///
/// Returns true if the foreground process is not the shell itself
/// (e.g., running vim, cargo, python, etc.)
bool terminal_pool_has_running_process(
    TerminalPoolHandle handle,
    size_t terminal_id
);

/// Check if terminal has Bracketed Paste Mode enabled
///
/// When enabled (app sent \x1b[?2004h), paste should be wrapped with escape sequences.
/// When disabled, send raw text directly.
///
/// Returns:
/// - true: Bracketed Paste Mode is enabled, need to wrap with \x1b[200~ and \x1b[201~
/// - false: Not enabled, send raw text
bool terminal_pool_is_bracketed_paste_enabled(
    TerminalPoolHandle handle,
    size_t terminal_id
);

/// Check if terminal has Kitty keyboard protocol enabled
///
/// Apps enable Kitty keyboard mode by sending `CSI > flags u`.
/// When enabled, keyboard input should use Kitty protocol encoding.
///
/// Returns:
/// - true: Kitty keyboard protocol enabled, use key_to_escape_sequence_with_mode(key, mods, 1)
/// - false: Use traditional Xterm encoding, use key_to_escape_sequence(key, mods)
///
/// Example:
///   if (terminal_pool_is_kitty_keyboard_enabled(pool, tid)) {
///       const char* seq = key_to_escape_sequence_with_mode(keyCode, mods, 1);  // Kitty
///   } else {
///       const char* seq = key_to_escape_sequence(keyCode, mods);  // Xterm
///   }
bool terminal_pool_is_kitty_keyboard_enabled(
    TerminalPoolHandle handle,
    size_t terminal_id
);

/// Check if terminal has Mouse Tracking Mode enabled (SGR 1006)
///
/// Apps enable mouse tracking by sending `\x1b[?1006h` (SGR mode).
/// When enabled, mouse events should be sent to the PTY instead of
/// being handled by the terminal (selection, scrollback, etc).
///
/// Returns:
/// - true: Mouse tracking enabled, send SGR mouse reports to PTY
/// - false: Mouse tracking disabled, handle mouse events locally
bool terminal_pool_has_mouse_tracking_mode(
    TerminalPoolHandle handle,
    size_t terminal_id
);

/// Send SGR format mouse report to PTY
///
/// SGR mouse report format: `\x1b[<button;col;rowM` (pressed) or `\x1b[<button;col;rowm` (released)
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
/// @param button Button code:
///   - 0=left, 1=middle, 2=right
///   - 64=scroll up, 65=scroll down
/// @param col Grid column (1-based)
/// @param row Grid row (1-based)
/// @param pressed true for press (M), false for release (m)
/// @return true on success, false if terminal not found
bool terminal_pool_send_mouse_sgr(
    TerminalPoolHandle handle,
    size_t terminal_id,
    uint8_t button,
    uint16_t col,
    uint16_t row,
    bool pressed
);

/// Resize terminal
bool terminal_pool_resize_terminal(
    TerminalPoolHandle handle,
    size_t terminal_id,
    uint16_t cols,
    uint16_t rows,
    float width,
    float height
);

/// Send input to terminal
bool terminal_pool_input(
    TerminalPoolHandle handle,
    size_t terminal_id,
    const uint8_t* data,
    size_t len
);

/// Scroll terminal
bool terminal_pool_scroll(
    TerminalPoolHandle handle,
    size_t terminal_id,
    int32_t delta
);

// ===== Render Flow (Unified Submit) =====

/// Begin new frame (clear pending objects)
void terminal_pool_begin_frame(TerminalPoolHandle handle);

/// Render terminal at position (accumulate to pending list)
///
/// Parameters:
/// - terminal_id: Terminal to render
/// - x, y: Position (logical coordinates, Y from top)
/// - width, height: Terminal area size (logical coordinates)
///   - If > 0, auto-calculate cols/rows and resize
///   - If = 0, don't resize (keep current size)
bool terminal_pool_render_terminal(
    TerminalPoolHandle handle,
    size_t terminal_id,
    float x,
    float y,
    float width,
    float height
);

/// End frame (unified submit to GPU)
void terminal_pool_end_frame(TerminalPoolHandle handle);

/// Resize Sugarloaf render surface
void terminal_pool_resize_sugarloaf(
    TerminalPoolHandle handle,
    float width,
    float height
);

/// Set DPI scale (call when window moves between screens with different DPI)
///
/// Updates Rust-side scale factor to ensure:
/// - Correct font metrics calculation
/// - Correct selection coordinate conversion
/// - Correct render position calculation
void terminal_pool_set_scale(
    TerminalPoolHandle handle,
    float scale
);

/// Set event callback
void terminal_pool_set_event_callback(
    TerminalPoolHandle handle,
    TerminalPoolEventCallback callback,
    void* context
);

/// Set string event callback
///
/// Use this callback to receive events with string payloads
/// (CurrentDirectoryChanged, CommandExecuted)
void terminal_pool_set_string_event_callback(
    TerminalPoolHandle handle,
    TerminalPoolStringEventCallback callback,
    void* context
);

/// Get terminal count
size_t terminal_pool_terminal_count(TerminalPoolHandle handle);

/// Check if needs render
bool terminal_pool_needs_render(TerminalPoolHandle handle);

/// Clear render flag
void terminal_pool_clear_render_flag(TerminalPoolHandle handle);

// =============================================================================
// Selection API (new architecture)
// =============================================================================

/// Screen to absolute coordinate result
typedef struct {
    int64_t absolute_row;
    size_t col;
    bool success;
} ScreenToAbsoluteResult;

/// Convert screen coordinates to absolute coordinates
ScreenToAbsoluteResult terminal_pool_screen_to_absolute(
    TerminalPoolHandle handle,
    size_t terminal_id,
    size_t screen_row,
    size_t screen_col
);

/// Set selection
bool terminal_pool_set_selection(
    TerminalPoolHandle handle,
    size_t terminal_id,
    int64_t start_absolute_row,
    size_t start_col,
    int64_t end_absolute_row,
    size_t end_col
);

/// Clear selection
bool terminal_pool_clear_selection(
    TerminalPoolHandle handle,
    size_t terminal_id
);

/// Finalize selection result
typedef struct {
    char* text;             // Selected text (UTF-8, must be freed with terminal_pool_free_string)
    size_t text_len;        // Text length (without null terminator)
    bool has_selection;     // Whether there is a valid selection (non-whitespace content)
} FinalizeSelectionResult;

/// Finalize selection (call on mouseUp)
///
/// Business logic:
/// - Check if selection content is all whitespace
/// - If all whitespace, auto-clear selection, return has_selection=false
/// - If has content, keep selection, return selected text
///
/// Caller must free text with terminal_pool_free_string
FinalizeSelectionResult terminal_pool_finalize_selection(
    TerminalPoolHandle handle,
    size_t terminal_id
);

/// Free string returned by finalize_selection or get_selection_text
void terminal_pool_free_string(char* ptr);

/// Get selection text result
typedef struct {
    char* text;             // Selected text (UTF-8, must be freed with terminal_pool_free_string)
    size_t text_len;        // Text length (without null terminator)
    bool success;           // Whether successful
} GetSelectionTextResult;

/// Get selected text (without clearing selection)
///
/// Used for Cmd+C copy etc.
///
/// Caller must free text with terminal_pool_free_string
GetSelectionTextResult terminal_pool_get_selection_text(
    TerminalPoolHandle handle,
    size_t terminal_id
);

/// Get font metrics from TerminalPool (DDD architecture)
///
/// Returns font metrics consistent with rendering:
/// - cell_width: Cell width (physical pixels)
/// - cell_height: Base cell height (physical pixels, without line_height_factor)
/// - line_height: Actual line height (physical pixels, = cell_height * line_height_factor)
///
/// Note: Mouse coordinate conversion should use line_height (not cell_height)
bool terminal_pool_get_font_metrics(
    TerminalPoolHandle handle,
    SugarloafFontMetrics* out_metrics
);

/// Change font size
///
/// @param handle TerminalPool handle
/// @param operation 0=reset(14pt), 1=decrease(-1pt), 2=increase(+1pt)
/// @return true if successful, false if handle is invalid
bool terminal_pool_change_font_size(
    TerminalPoolHandle handle,
    uint8_t operation
);

/// Get current font size
///
/// @param handle TerminalPool handle
/// @return Current font size in pt, or 0.0 if handle is invalid
float terminal_pool_get_font_size(
    TerminalPoolHandle handle
);

// =============================================================================
// Search API
// =============================================================================

/// Search for text in terminal
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
/// @param query Search query (UTF-8 string)
/// @return Number of matches (>= 0), or -1 on failure
int32_t terminal_pool_search(
    TerminalPoolHandle handle,
    size_t terminal_id,
    const char* query
);

/// Jump to next search match
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
void terminal_pool_search_next(
    TerminalPoolHandle handle,
    size_t terminal_id
);

/// Jump to previous search match
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
void terminal_pool_search_prev(
    TerminalPoolHandle handle,
    size_t terminal_id
);

/// Clear search
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
void terminal_pool_clear_search(
    TerminalPoolHandle handle,
    size_t terminal_id
);

// =============================================================================
// Cursor & Word Boundary API (new architecture)
// =============================================================================

/// Cursor position (screen coordinates)
typedef struct {
    uint16_t col;       // Column (0-based)
    uint16_t row;       // Row (0-based, relative to visible area)
    bool valid;         // Whether the result is valid
} FFICursorPosition;

/// Word boundary information
typedef struct {
    uint16_t start_col;     // Start column (screen coordinates)
    uint16_t end_col;       // End column (screen coordinates, inclusive)
    int64_t absolute_row;   // Absolute row number
    char* text_ptr;         // Word text (must be freed with terminal_pool_free_word_boundary)
    size_t text_len;        // Text length in bytes
    bool valid;             // Whether the result is valid
} FFIWordBoundary;

/// Get cursor position
///
/// Returns the cursor position in screen coordinates (relative to visible area).
/// If the terminal is scrolling through history, the cursor may not be visible.
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
/// @return Cursor position, valid=false if terminal not found
FFICursorPosition terminal_pool_get_cursor(
    TerminalPoolHandle handle,
    size_t terminal_id
);

/// Get word boundary at specified position
///
/// Word segmentation rules (similar to Swift WordBoundaryDetector):
/// 1. CJK characters: consecutive CJK characters form one word
/// 2. Alphanumeric/underscore: consecutive characters form one word
/// 3. Whitespace: acts as separator
/// 4. Other symbols: form individual words
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
/// @param screen_row Screen row (0-based, relative to visible area)
/// @param screen_col Screen column (0-based)
/// @return Word boundary, valid=false if terminal not found or position invalid
///         If valid=true, text_ptr must be freed with terminal_pool_free_word_boundary
FFIWordBoundary terminal_pool_get_word_at(
    TerminalPoolHandle handle,
    int32_t terminal_id,
    int32_t screen_row,
    int32_t screen_col
);

/// Free word boundary resources
///
/// @param boundary Word boundary returned by terminal_pool_get_word_at
///
/// Note: Only call this for valid=true boundaries, do not free the same boundary twice
void terminal_pool_free_word_boundary(FFIWordBoundary boundary);

// =============================================================================
// Hyperlink API
// =============================================================================

/// Hyperlink query result (C ABI compatible)
typedef struct {
    int64_t start_row;      // Start row (absolute coordinates)
    uint16_t start_col;     // Start column
    int64_t end_row;        // End row (absolute coordinates)
    uint16_t end_col;       // End column
    char* uri_ptr;          // URI pointer (must be freed with terminal_pool_free_hyperlink)
    size_t uri_len;         // URI length in bytes
    bool valid;             // Whether result is valid (true = has hyperlink)
} FFIHyperlink;

/// Get hyperlink at specified position
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
/// @param screen_row Screen row (0-based)
/// @param screen_col Screen column (0-based)
/// @return Hyperlink info, valid=false if no hyperlink at position
///         If valid=true, uri_ptr must be freed with terminal_pool_free_hyperlink
FFIHyperlink terminal_pool_get_hyperlink_at(
    TerminalPoolHandle handle,
    int32_t terminal_id,
    int32_t screen_row,
    int32_t screen_col
);

/// Free hyperlink resources
///
/// @param hyperlink Hyperlink returned by terminal_pool_get_hyperlink_at
///
/// Note: Only call this for valid=true hyperlinks, do not free the same hyperlink twice
void terminal_pool_free_hyperlink(FFIHyperlink hyperlink);

/// Set hyperlink hover state (triggers highlight rendering)
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
/// @param start_row Start row (absolute coordinates)
/// @param start_col Start column
/// @param end_row End row (absolute coordinates)
/// @param end_col End column
/// @param uri Hyperlink URI (C string)
/// @return true if successful
bool terminal_pool_set_hyperlink_hover(
    TerminalPoolHandle handle,
    int32_t terminal_id,
    int64_t start_row,
    uint16_t start_col,
    int64_t end_row,
    uint16_t end_col,
    const char* uri
);

/// Clear hyperlink hover state
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
/// @return true if successful
bool terminal_pool_clear_hyperlink_hover(
    TerminalPoolHandle handle,
    int32_t terminal_id
);

/// Get auto-detected URL at specified position
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
/// @param screen_row Screen row (0-based)
/// @param screen_col Screen column (0-based)
/// @return URL info, valid=false if no URL at position
///         If valid=true, uri_ptr must be freed with terminal_pool_free_hyperlink
FFIHyperlink terminal_pool_get_url_at(
    TerminalPoolHandle handle,
    int32_t terminal_id,
    int32_t screen_row,
    int32_t screen_col
);

// =============================================================================
// IME Preedit API
// =============================================================================

/// Set IME preedit state
///
/// Display preedit text (e.g., pinyin "nihao") at the current cursor position.
/// Rust side will get the absolute cursor coordinates from Terminal.
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
/// @param text Preedit text (UTF-8 C string)
/// @param cursor_offset Cursor position within preedit (character index, not bytes)
/// @return true if successful
bool terminal_pool_set_ime_preedit(
    TerminalPoolHandle handle,
    int32_t terminal_id,
    const char* text,
    uint32_t cursor_offset
);

/// Clear IME preedit state
///
/// Should be called when:
/// - User confirms input (commitText)
/// - User cancels input (cancelComposition)
/// - Terminal switches
/// - Terminal loses focus
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
/// @return true if successful
bool terminal_pool_clear_ime_preedit(
    TerminalPoolHandle handle,
    int32_t terminal_id
);

// =============================================================================
// RenderScheduler API (CVDisplayLink in Rust)
// =============================================================================

/// RenderScheduler handle (opaque pointer)
typedef void* RenderSchedulerHandle;

/// Render layout info
typedef struct {
    size_t terminal_id;
    float x;
    float y;
    float width;
    float height;
} RenderLayout;

/// Render callback type
///
/// Called on VSync, Swift should execute render in callback:
/// - terminal_pool_begin_frame
/// - terminal_pool_render_terminal (for each layout item)
/// - terminal_pool_end_frame
typedef void (*RenderSchedulerCallback)(
    void* context,
    const RenderLayout* layout,
    size_t layout_count
);

/// Create RenderScheduler
RenderSchedulerHandle render_scheduler_create(void);

/// Destroy RenderScheduler
void render_scheduler_destroy(RenderSchedulerHandle handle);

/// Set render callback
///
/// Callback is called on CVDisplayLink VSync
void render_scheduler_set_callback(
    RenderSchedulerHandle handle,
    RenderSchedulerCallback callback,
    void* context
);

/// Start RenderScheduler (start CVDisplayLink)
bool render_scheduler_start(RenderSchedulerHandle handle);

/// Stop RenderScheduler
void render_scheduler_stop(RenderSchedulerHandle handle);

/// Request render (mark dirty)
void render_scheduler_request_render(RenderSchedulerHandle handle);

/// Set render layout
///
/// Layout info will be passed to callback on next VSync
void render_scheduler_set_layout(
    RenderSchedulerHandle handle,
    const RenderLayout* layout,
    size_t count
);

/// Bind to TerminalPool (new architecture)
///
/// After binding:
/// - RenderScheduler and TerminalPool share needs_render flag
/// - RenderScheduler calls pool.render_all() on VSync
/// - No Swift involvement in render loop
void render_scheduler_bind_to_pool(
    RenderSchedulerHandle scheduler_handle,
    TerminalPoolHandle pool_handle
);

// ============================================================================
// New Architecture: Rust-side rendering
// ============================================================================

/// Terminal render layout info (new architecture)
typedef struct {
    size_t terminal_id;
    float x;
    float y;
    float width;
    float height;
} TerminalRenderLayout;

/// Set render layout (new architecture)
///
/// Swift calls this when layout changes (tab switch, window resize, etc.)
/// Rust uses this layout for rendering on VSync
///
/// Note: Coordinates should be in Rust coordinate system (Y from top)
void terminal_pool_set_render_layout(
    TerminalPoolHandle handle,
    const TerminalRenderLayout* layout,
    size_t count,
    float container_height
);

/// Trigger a full render (new architecture)
///
/// Usually not needed, RenderScheduler calls this automatically on VSync
/// This is for special cases (initialization, force refresh)
void terminal_pool_render_all(TerminalPoolHandle handle);

// ============================================================================
// Terminal Mode API
// ============================================================================

/// Set terminal mode
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
/// @param mode Terminal mode (0=Active, 1=Background)
///
/// - Active: Full processing + render callbacks
/// - Background: Full VTE parsing but no render callbacks (save CPU/GPU)
/// - Switching to Active triggers a render refresh
void terminal_pool_set_mode(
    TerminalPoolHandle handle,
    size_t terminal_id,
    uint8_t mode
);

/// Get terminal mode
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
/// @return Terminal mode (0=Active, 1=Background, 255=invalid)
uint8_t terminal_pool_get_mode(
    TerminalPoolHandle handle,
    size_t terminal_id
);

// ============================================================================
// Lock-Free Cache API (Phase 1 Async FFI)
// ============================================================================
// These functions read from atomic caches, never block the main thread.
// Data is a snapshot from the last render, may have slight delay.

/// Selection range result (lock-free)
typedef struct {
    int32_t start_row;      // Start row (absolute row number)
    uint32_t start_col;     // Start column
    int32_t end_row;        // End row (absolute row number)
    uint32_t end_col;       // End column
    bool has_selection;     // Whether there is a valid selection
} SelectionRange;

/// Get selection range (lock-free)
///
/// Reads from atomic cache, no terminal lock required.
/// Main thread safe, never blocks.
///
/// Note: Returns snapshot from last render, may differ slightly from real-time state.
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
/// @return Selection range, has_selection=false if no selection or terminal not found
SelectionRange terminal_pool_get_selection_range(
    TerminalPoolHandle handle,
    size_t terminal_id
);

/// Scroll info result (lock-free)
typedef struct {
    uint32_t display_offset;    // Current scroll position
    uint16_t history_size;      // History line count
    uint16_t total_lines;       // Total line count
    bool valid;                 // Whether the result is valid
} ScrollInfo;

/// Get scroll info (lock-free)
///
/// Reads from atomic cache, no terminal lock required.
/// Main thread safe, never blocks.
///
/// Note: Returns snapshot from last render, may differ slightly from real-time state.
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
/// @return Scroll info, valid=false if terminal not found
ScrollInfo terminal_pool_get_scroll_info(
    TerminalPoolHandle handle,
    size_t terminal_id
);

// =============================================================================
// Keyboard API (key to escape sequence conversion)
// =============================================================================

/// Keyboard encoding mode
typedef enum {
    /// Xterm traditional mode (default)
    /// Some key combinations (like Shift+Enter) cannot be distinguished
    KeyboardMode_Xterm = 0,
    /// Kitty keyboard protocol
    /// All key+modifier combinations have unique escape sequences
    KeyboardMode_Kitty = 1,
} KeyboardMode;

/// Convert keyboard event to terminal escape sequence (Xterm mode)
///
/// @param key_code macOS keyCode (NSEvent.keyCode)
/// @param modifiers Modifier key flags:
///   - bit 0: Shift
///   - bit 1: Control
///   - bit 2: Option (Alt)
///   - bit 3: Command (Meta)
///
/// @return Escape sequence string on success (must be freed with free_key_sequence),
///         NULL if not a special key (caller should use character input)
///
/// Example:
///   const char* seq = key_to_escape_sequence(126, 1);  // Shift+Up -> "\x1b[1;2A"
///   if (seq) {
///       write_to_terminal(seq);
///       free_key_sequence(seq);
///   }
const char* key_to_escape_sequence(uint16_t key_code, uint32_t modifiers);

/// Convert keyboard event to terminal escape sequence (with encoding mode)
///
/// @param key_code macOS keyCode (NSEvent.keyCode)
/// @param modifiers Modifier key flags
/// @param mode Encoding mode (0 = Xterm, 1 = Kitty)
///
/// @return Escape sequence string on success (must be freed with free_key_sequence),
///         NULL if not a special key
///
/// Kitty mode examples:
///   Shift+Enter -> "\x1b[13;2u"  (CSI 13 ; 2 u)
///   Ctrl+Enter  -> "\x1b[13;5u"  (CSI 13 ; 5 u)
///   Shift+Tab   -> "\x1b[9;2u"   (CSI 9 ; 2 u)
const char* key_to_escape_sequence_with_mode(uint16_t key_code, uint32_t modifiers, uint8_t mode);

/// Free string returned by key_to_escape_sequence
///
/// @param ptr Pointer returned by key_to_escape_sequence (NULL is safe)
void free_key_sequence(const char* ptr);

// =============================================================================
// Logging API (Rust to Swift log bridging)
// =============================================================================

/// Rust log level (matches RustLogLevel in logging.rs)
typedef enum {
    RustLogLevel_Debug = 0,
    RustLogLevel_Info = 1,
    RustLogLevel_Warn = 2,
    RustLogLevel_Error = 3,
} RustLogLevel;

/// Log callback type
///
/// Swift should implement this callback and set it via set_rust_log_callback
/// to receive logs from Rust side.
///
/// @param level Log level
/// @param message Log message (UTF-8 C string)
///
/// Note: Callback may be called from multiple threads, must be thread-safe
typedef void (*RustLogCallback)(RustLogLevel level, const char* message);

/// Set log callback
///
/// Swift should call this during app startup to receive Rust logs.
/// Logs will be forwarded to LogManager for persistence.
///
/// @param callback Log callback function
///
/// Example (Swift):
///   let callback: RustLogCallback = { level, message in
///       guard let message = message else { return }
///       let text = String(cString: message)
///       switch level {
///       case RustLogLevel_Warn:
///           LogManager.shared.warn(text)
///       case RustLogLevel_Error:
///           LogManager.shared.error(text)
///       default:
///           break
///       }
///   }
///   set_rust_log_callback(callback)
void set_rust_log_callback(RustLogCallback callback);

/// Clear log callback (usually not needed)
void clear_rust_log_callback(void);

// =============================================================================
// Terminal Migration API (Cross-window move)
// =============================================================================

/// DetachedTerminal handle (opaque pointer)
///
/// Used to transfer terminals between pools
typedef void* DetachedTerminalHandle;

/// Detach terminal (for cross-pool migration)
///
/// Removes terminal from current pool and returns a DetachedTerminal handle.
/// PTY connection stays alive, terminal state is fully preserved.
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID to detach
/// @return DetachedTerminal handle on success, NULL on failure (terminal not found)
///
/// Note:
/// - Returned handle must be passed to terminal_pool_attach_terminal
/// - Or destroyed with detached_terminal_destroy
DetachedTerminalHandle terminal_pool_detach_terminal(
    TerminalPoolHandle handle,
    size_t terminal_id
);

/// Attach detached terminal (for cross-pool migration)
///
/// Adds DetachedTerminal to current pool.
///
/// @param handle TerminalPool handle (target pool)
/// @param detached DetachedTerminal handle
/// @return Terminal ID in target pool (>= 1) on success, -1 on failure
///
/// Note:
/// - After calling, detached handle is no longer valid
/// - Terminal will use original ID if no conflict, otherwise new ID
int64_t terminal_pool_attach_terminal(
    TerminalPoolHandle handle,
    DetachedTerminalHandle detached
);

/// Destroy detached terminal (without migration, closes PTY)
///
/// If detached terminal doesn't need migration, use this to release resources.
///
/// @param detached DetachedTerminal handle
void detached_terminal_destroy(DetachedTerminalHandle detached);

/// Get original ID of detached terminal
///
/// @param detached DetachedTerminal handle
/// @return Terminal's original ID on success, -1 on failure
int64_t detached_terminal_get_id(DetachedTerminalHandle detached);

// =============================================================================
// Terminal Snapshot APIs (for Session Recording)
// =============================================================================

/// Get visible lines of terminal (for snapshot recording)
///
/// Returns the text content of the visible area, one line per string.
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
/// @param out_lines Output parameter - array of C strings (caller must free with terminal_pool_free_string_array)
/// @param out_count Output parameter - number of lines
/// @return true on success, false on failure
bool terminal_pool_get_visible_lines(
    TerminalPoolHandle handle,
    int64_t terminal_id,
    const char*** out_lines,
    size_t* out_count
);

/// Get cursor position (for snapshot recording)
///
/// Returns the cursor position relative to the visible area.
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
/// @param out_row Output parameter - cursor row (0-based, relative to visible area)
/// @param out_col Output parameter - cursor column (0-based)
/// @return true on success, false on failure
bool terminal_pool_get_cursor_position(
    TerminalPoolHandle handle,
    int64_t terminal_id,
    int32_t* out_row,
    int32_t* out_col
);

/// Get scrollback buffer line count (for snapshot recording)
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
/// @return Number of scrollback lines on success, -1 on failure
int64_t terminal_pool_get_scrollback_lines(
    TerminalPoolHandle handle,
    int64_t terminal_id
);

/// Free string array allocated by terminal_pool_get_visible_lines
///
/// @param lines String array pointer
/// @param count Array length
void terminal_pool_free_string_array(
    const char** lines,
    size_t count
);

// =============================================================================
// LogBuffer API (Optional, only available when log_buffer_size > 0)
// =============================================================================

/// Query terminal log buffer
///
/// Only available when `log_buffer_size > 0` was set during pool creation.
/// Returns JSON string with log query result containing:
/// - lines: array of {seq, text} objects
/// - next_seq: next sequence number for pagination
/// - has_more: whether more lines exist
/// - truncated: whether old logs were discarded
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
/// @param since Return lines with seq > since (0 for all)
/// @param limit Maximum number of lines to return
/// @param search Optional search filter (NULL for no filter)
/// @param is_regex If true, treat search as a regex pattern
/// @param case_insensitive If true, search is case-insensitive
/// @return JSON string on success (must be freed with rio_free_string), NULL if disabled or error
char* terminal_pool_query_log(
    TerminalPoolHandle handle,
    size_t terminal_id,
    uint64_t since,
    size_t limit,
    const char* search,
    bool is_regex,
    bool case_insensitive
);

/// Get last N lines from terminal log buffer
///
/// Only available when `log_buffer_size > 0` was set during pool creation.
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
/// @param count Number of lines to return
/// @return JSON array string on success (must be freed with rio_free_string), NULL if disabled or error
char* terminal_pool_tail_log(
    TerminalPoolHandle handle,
    size_t terminal_id,
    size_t count
);

/// Clear terminal log buffer
///
/// Only available when `log_buffer_size > 0` was set during pool creation.
///
/// @param handle TerminalPool handle
/// @param terminal_id Terminal ID
/// @return true on success, false if disabled or terminal not found
bool terminal_pool_clear_log(
    TerminalPoolHandle handle,
    size_t terminal_id
);

#endif /* SugarloafBridge_h */
