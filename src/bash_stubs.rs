//! Stub Bash/readline symbols for the standalone `flyline-standalone` binary.
//!
//! The library is normally loaded into Bash and resolves these at runtime. The
//! standalone editor links the same rlib without a host shell, so we provide
//! inert definitions that satisfy the linker. Zsh mode routes through
//! `shell::zsh::ZSH_BACKEND` and must not call into real Bash FFI.

#![allow(non_snake_case, dead_code, static_mut_refs)]

use crate::bash_symbols::{
    Alias, BashBuiltinType, BashInput, BufferedStream, CompSpec, FunctionDef, HistoryEntry,
    ShellVar, StreamSaver, StreamType,
};
use libc::{c_char, c_int, c_uint, c_void, pid_t};

macro_rules! stub_fn_ret {
    ($name:ident($($arg:ident: $ty:ty),*) -> $ret:ty = $val:expr) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name($($arg: $ty),*) -> $ret {
            let _ = ($($arg),*);
            $val
        }
    };
    ($name:ident($($arg:ident: $ty:ty),*) -> $ret:ty) => {
        stub_fn_ret!($name($($arg: $ty),*) -> $ret = Default::default());
    };
    ($name:ident($($arg:ident: $ty:ty),*)) => {
        stub_fn_ret!($name($($arg: $ty),*) -> () = ());
    };
}

fn dup_cstr(s: *const c_char) -> *mut c_char {
    if s.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        let len = libc::strlen(s);
        let ptr = libc::malloc(len + 1) as *mut c_char;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        std::ptr::copy_nonoverlapping(s, ptr, len + 1);
        ptr
    }
}

// --- statics ----------------------------------------------------------------

#[unsafe(no_mangle)]
pub static mut stream_list: *mut StreamSaver = std::ptr::null_mut();

#[unsafe(no_mangle)]
pub static mut bash_input: BashInput = BashInput {
    stream_type: StreamType::None,
    name: std::ptr::null_mut(),
    location: crate::bash_symbols::InputStreamLocation {
        string: std::ptr::null_mut(),
    },
    getter: None,
    ungetter: None,
};

#[unsafe(no_mangle)]
pub static interactive: c_int = 1;
#[unsafe(no_mangle)]
pub static interactive_shell: c_int = 1;
#[unsafe(no_mangle)]
pub static no_line_editing: c_int = 0;

#[unsafe(no_mangle)]
pub static mut rl_line_buffer: *mut c_char = std::ptr::null_mut();
#[unsafe(no_mangle)]
pub static mut rl_point: c_int = 0;
#[unsafe(no_mangle)]
pub static mut rl_end: c_int = 0;
#[unsafe(no_mangle)]
pub static mut rl_completion_found_quote: c_int = 0;
#[unsafe(no_mangle)]
pub static mut rl_completion_quote_character: c_int = 0;
#[unsafe(no_mangle)]
pub static mut rl_filename_quoting_desired: c_int = 0;
#[unsafe(no_mangle)]
pub static mut rl_filename_completion_desired: c_int = 0;
#[cfg(not(feature = "pre_bash_4_4"))]
#[unsafe(no_mangle)]
pub static mut rl_completion_suppress_append: c_int = 0;
#[unsafe(no_mangle)]
pub static mut rl_completion_append_character: c_int = b' ' as c_int;
#[cfg(not(feature = "pre_bash_4_4"))]
#[unsafe(no_mangle)]
pub static mut rl_sort_completion_matches: c_int = 1;
#[unsafe(no_mangle)]
pub static mut rl_filename_dequoting_function: Option<
    extern "C" fn(*const c_char, c_int) -> *mut c_char,
> = None;
#[unsafe(no_mangle)]
pub static mut rl_filename_quoting_function: Option<
    extern "C" fn(*const c_char, c_int, *const c_char) -> *mut c_char,
> = None;

#[unsafe(no_mangle)]
pub static mut shell_builtins: *mut BashBuiltinType = std::ptr::null_mut();
#[unsafe(no_mangle)]
pub static mut num_shell_builtins: c_int = 0;
#[unsafe(no_mangle)]
pub static mut rl_readline_state: libc::c_ulong = 0;
#[unsafe(no_mangle)]
pub static mut current_command_line_count: c_int = 0;
#[unsafe(no_mangle)]
pub static mut current_readline_prompt: *mut c_char = std::ptr::null_mut();
#[unsafe(no_mangle)]
pub static mut terminating_signal: c_int = 0;
#[cfg(not(feature = "pre_bash_4_4"))]
#[unsafe(no_mangle)]
pub static mut rl_signal_event_hook: Option<extern "C" fn()> = None;
#[unsafe(no_mangle)]
pub static mut job_control: c_int = 0;
#[unsafe(no_mangle)]
pub static mut shell_pgrp: pid_t = 0;
#[unsafe(no_mangle)]
pub static mut last_command_exit_value: c_int = 0;
#[unsafe(no_mangle)]
pub static mut current_host_name: *mut c_char = std::ptr::null_mut();
#[unsafe(no_mangle)]
pub static mut array_needs_making: c_int = 0;

// --- functions --------------------------------------------------------------

stub_fn_ret!(push_stream(reset_lineno: c_int));
stub_fn_ret!(pop_stream());
stub_fn_ret!(with_input_from_stdin());
stub_fn_ret!(get_alias_value(_name: *const c_char) -> *mut c_char = std::ptr::null_mut());
stub_fn_ret!(find_function_def(_name: *const c_char) -> *mut FunctionDef = std::ptr::null_mut());
stub_fn_ret!(describe_command(_command: *const c_char, _dflags: c_int) -> c_int = 1);
stub_fn_ret!(
    programmable_completions(
        _cmd: *const c_char,
        _word: *const c_char,
        _start: c_int,
        _end: c_int,
        _foundp: *mut c_int
    ) -> *mut *mut c_char = std::ptr::null_mut()
);
stub_fn_ret!(progcomp_search(_cmd: *const c_char) -> *mut CompSpec = std::ptr::null_mut());
stub_fn_ret!(
    bash_default_completion(
        _text: *const c_char,
        _start: c_int,
        _end: c_int,
        _qc: c_int,
        _compflags: c_int
    ) -> *mut *mut c_char = std::ptr::null_mut()
);
#[cfg(not(feature = "pre_bash_4_4"))]
stub_fn_ret!(pcomp_set_readline_variables(_flags: c_int, _nval: c_int));
stub_fn_ret!(all_aliases() -> *mut *mut Alias = std::ptr::null_mut());
stub_fn_ret!(
    all_variables_matching_prefix(_prefix: *const c_char) -> *mut *mut c_char =
        std::ptr::null_mut()
);
stub_fn_ret!(all_shell_functions() -> *mut *mut ShellVar = std::ptr::null_mut());
stub_fn_ret!(history_list() -> *mut *mut HistoryEntry = std::ptr::null_mut());
stub_fn_ret!(find_variable(_name: *const c_char) -> *mut ShellVar = std::ptr::null_mut());
stub_fn_ret!(bind_variable(_name: *const c_char, _value: *const c_char, _flags: c_int) -> *mut ShellVar = std::ptr::null_mut());
stub_fn_ret!(unbind_variable(_name: *const c_char) -> c_int = 0);
#[cfg(not(feature = "pre_bash_4_4"))]
stub_fn_ret!(evalstring(_string: *mut c_char, _from_file: *const c_char, _flags: c_int) -> c_int = 0);
#[cfg(feature = "pre_bash_4_4")]
stub_fn_ret!(parse_and_execute(_string: *mut c_char, _from_file: *const c_char, _flags: c_int) -> c_int = 0);
stub_fn_ret!(termsig_handler(_sig: c_int));
stub_fn_ret!(give_terminal_to(_pgrp: pid_t, _force: c_int) -> c_int = 0);
stub_fn_ret!(get_working_directory(_for_whom: *const c_char) -> *mut c_char = std::ptr::null_mut());
stub_fn_ret!(show_var_attributes(_var: *mut ShellVar, _flags: c_int, _output_fd: c_int) -> c_int = 0);

// NOTE: do NOT stub `getenv`. A `#[no_mangle] getenv` here interposes the libc
// symbol for the whole process, and calling `libc::getenv` from it resolves
// straight back to itself — infinite tail-recursion that spins at 100% CPU and
// wedges the shell (Rust's `std::env::var` calls `getenv`). The bash_symbols
// extern reference is satisfied by the real libc `getenv` at link time.

#[cfg(not(feature = "pre_bash_4_4"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn decode_prompt_string(
    string: *const c_char,
    _is_prompt: c_int,
) -> *mut c_char {
    dup_cstr(string)
}

#[cfg(feature = "pre_bash_4_4")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn decode_prompt_string(string: *const c_char) -> *mut c_char {
    dup_cstr(string)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn expand_string_to_string(
    string: *const c_char,
    _quoted: c_int,
) -> *mut c_char {
    dup_cstr(string)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmalloc(size: libc::size_t) -> *mut c_void {
    unsafe { libc::malloc(size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn xrealloc(ptr: *mut c_void, size: libc::size_t) -> *mut c_void {
    unsafe { libc::realloc(ptr, size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn xfree(ptr: *mut c_void) {
    if !ptr.is_null() {
        unsafe { libc::free(ptr) };
    }
}

// Silence unused import warnings from struct fields in static initializers.
const _: () = {
    let _ = std::mem::size_of::<BufferedStream>();
    let _ = std::mem::size_of::<c_uint>();
};
