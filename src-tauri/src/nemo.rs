//! Adapter for NVIDIA's NeMo-Speech.cpp ASR C SDK.
//!
//! The SDK is loaded at runtime so the app remains buildable without a GPU SDK
//! or the NeMo C++ toolchain. Windows CPU SDK DLLs are discovered beside the
//! app, in the standard NeMoSpeech install, on PATH, or through
//! `NEMO_SPEECH_SDK_DIR`.

#[cfg(windows)]
mod platform {
    use std::env;
    use std::ffi::{CStr, CString, c_char, c_void};
    use std::mem;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::ptr;
    use std::sync::Arc;

    type Module = *mut c_void;
    type DirectoryCookie = *mut c_void;

    const LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR: u32 = 0x0000_0100;
    const LOAD_LIBRARY_SEARCH_USER_DIRS: u32 = 0x0000_0400;
    const LOAD_LIBRARY_SEARCH_DEFAULT_DIRS: u32 = 0x0000_1000;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AddDllDirectory(new_directory: *const u16) -> DirectoryCookie;
        fn RemoveDllDirectory(cookie: DirectoryCookie) -> i32;
        fn LoadLibraryExW(file_name: *const u16, file: *mut c_void, flags: u32) -> Module;
        fn GetProcAddress(module: Module, name: *const u8) -> *mut c_void;
        fn FreeLibrary(module: Module) -> i32;
        fn GetLastError() -> u32;
    }

    #[repr(C)]
    struct BackendConfig {
        size: usize,
        gpu: i32,
    }

    #[repr(C)]
    struct ModelConfig {
        size: usize,
        path: *const c_char,
        name: *const c_char,
    }

    #[repr(C)]
    struct StreamingConfig {
        size: usize,
        chunk_size: f32,
        ctc_left_padding: f32,
        ctc_right_padding: f32,
        rnnt_right_context: i32,
    }

    // The C SDK documents these structs as append-only and accepts a shorter
    // size-prefixed struct, defaulting fields added after the supplied size.
    // These layouts cover the stable v1 fields used by the adapter.
    #[repr(C)]
    struct RecognizerConfig {
        size: usize,
        backend: *const BackendConfig,
        model: *const ModelConfig,
        streaming: *const c_void,
        decoder: *const c_void,
        vad: *const c_void,
        endpointing: *const c_void,
        postproc: *const c_void,
        diar: *const c_void,
    }

    #[repr(C)]
    struct RecognitionOptions {
        size: usize,
        request_id: *const c_char,
        language_code: *const c_char,
        interim_results: bool,
        enable_word_time_offsets: bool,
        enable_automatic_punctuation: bool,
        verbatim_transcripts: bool,
        profanity_filter: bool,
        stop_history_eou_ms: i32,
        speech_contexts: *const c_void,
        speech_context_count: usize,
        max_alternatives: i32,
        enable_speaker_diarization: bool,
        max_speaker_count: i32,
    }

    type CreateFn = unsafe extern "C" fn(*const RecognizerConfig, *mut *mut c_void) -> i32;
    type DestroyFn = unsafe extern "C" fn(*mut c_void);
    type StreamingRecognizeFn =
        unsafe extern "C" fn(*mut c_void, *const RecognitionOptions, *mut *mut c_void) -> i32;
    type StreamPushFn = unsafe extern "C" fn(*mut c_void, *const f32, usize, i32) -> i32;
    type StreamFinishFn = unsafe extern "C" fn(*mut c_void) -> i32;
    type StreamNextFn = unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> i32;
    type StreamCloseFn = unsafe extern "C" fn(*mut c_void);
    type ResultIsFinalFn = unsafe extern "C" fn(*const c_void) -> bool;
    type ResultTranscriptFn = unsafe extern "C" fn(*const c_void, usize) -> *const c_char;
    type LastErrorFn = unsafe extern "C" fn() -> *const c_char;

    struct Api {
        module: Module,
        directories: Vec<DirectoryCookie>,
        create: CreateFn,
        destroy: DestroyFn,
        streaming_recognize: StreamingRecognizeFn,
        stream_push: StreamPushFn,
        stream_finish: StreamFinishFn,
        stream_next: StreamNextFn,
        stream_close: StreamCloseFn,
        result_is_final: ResultIsFinalFn,
        result_transcript: ResultTranscriptFn,
        result_destroy: DestroyFn,
        last_error: LastErrorFn,
    }

    impl Drop for Api {
        fn drop(&mut self) {
            unsafe {
                if !self.module.is_null() {
                    FreeLibrary(self.module);
                }
                for cookie in self.directories.drain(..).rev() {
                    RemoveDllDirectory(cookie);
                }
            }
        }
    }

    struct ModuleGuard {
        module: Module,
        directories: Vec<DirectoryCookie>,
    }

    impl Drop for ModuleGuard {
        fn drop(&mut self) {
            unsafe {
                if !self.module.is_null() {
                    FreeLibrary(self.module);
                }
                for cookie in self.directories.drain(..).rev() {
                    RemoveDllDirectory(cookie);
                }
            }
        }
    }

    impl Api {
        fn load() -> Result<Arc<Self>, String> {
            let mut load_error = None;
            for directory in sdk_directories() {
                let library_path = ["nemo_speech_asr_c.dll", "libnemo_speech_asr_c.dll"]
                    .into_iter()
                    .map(|name| directory.join(name))
                    .find(|path| path.is_file());
                let Some(library_path) = library_path else {
                    continue;
                };

                let mut guard = ModuleGuard {
                    module: ptr::null_mut(),
                    directories: add_search_directories(&directory),
                };
                let wide_path: Vec<u16> = library_path
                    .as_os_str()
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                unsafe {
                    guard.module = LoadLibraryExW(
                        wide_path.as_ptr(),
                        ptr::null_mut(),
                        LOAD_LIBRARY_SEARCH_DEFAULT_DIRS
                            | LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR
                            | LOAD_LIBRARY_SEARCH_USER_DIRS,
                    );
                }
                if guard.module.is_null() {
                    let code = unsafe { GetLastError() };
                    load_error = Some(format!(
                        "Could not load {} (Windows error {code}).",
                        library_path.display()
                    ));
                    continue;
                }

                macro_rules! symbol {
                    ($name:literal, $ty:ty) => {{
                        unsafe {
                            load_symbol::<$ty>(guard.module, concat!($name, "\0").as_bytes())?
                        }
                    }};
                }

                let create = symbol!("nemo_speech_asr_create", CreateFn);
                let destroy = symbol!("nemo_speech_asr_destroy", DestroyFn);
                let streaming_recognize =
                    symbol!("nemo_speech_asr_streaming_recognize", StreamingRecognizeFn);
                let stream_push = symbol!("nemo_speech_asr_stream_push_f32", StreamPushFn);
                let stream_finish = symbol!("nemo_speech_asr_stream_finish", StreamFinishFn);
                let stream_next = symbol!("nemo_speech_asr_stream_next", StreamNextFn);
                let stream_close = symbol!("nemo_speech_asr_stream_close", StreamCloseFn);
                let result_is_final = symbol!("nemo_speech_asr_result_is_final", ResultIsFinalFn);
                let result_transcript =
                    symbol!("nemo_speech_asr_result_transcript", ResultTranscriptFn);
                let result_destroy = symbol!("nemo_speech_asr_result_destroy", DestroyFn);
                let last_error = symbol!("nemo_speech_asr_last_error", LastErrorFn);
                let module = guard.module;
                let directories = mem::take(&mut guard.directories);
                let api = Self {
                    module,
                    directories,
                    create,
                    destroy,
                    streaming_recognize,
                    stream_push,
                    stream_finish,
                    stream_next,
                    stream_close,
                    result_is_final,
                    result_transcript,
                    result_destroy,
                    last_error,
                };
                guard.module = ptr::null_mut();
                return Ok(Arc::new(api));
            }

            Err(load_error.unwrap_or_else(|| {
                "NeMo-Speech.cpp ASR C SDK was not found. Install its Windows CPU runtime or set NEMO_SPEECH_SDK_DIR.".into()
            }))
        }

        fn error(&self, operation: &str, status: i32) -> String {
            let detail = unsafe {
                let message = (self.last_error)();
                if message.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(message).to_string_lossy().trim().to_owned()
                }
            };
            if detail.is_empty() {
                format!("NeMo-Speech.cpp {operation} failed (status {status}).")
            } else {
                format!("NeMo-Speech.cpp {operation} failed: {detail}")
            }
        }
    }

    unsafe fn load_symbol<T: Copy>(module: Module, name: &[u8]) -> Result<T, String> {
        let address = unsafe { GetProcAddress(module, name.as_ptr()) };
        if address.is_null() {
            let symbol = CStr::from_bytes_with_nul(name)
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "unknown symbol".into());
            return Err(format!("NeMo-Speech.cpp SDK is missing {symbol}."));
        }
        Ok(unsafe { mem::transmute_copy(&address) })
    }

    fn sdk_directories() -> Vec<PathBuf> {
        let mut candidates = Vec::new();
        if let Some(root) = env::var_os("NEMO_SPEECH_SDK_DIR") {
            add_sdk_root(&mut candidates, PathBuf::from(root));
        }
        if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
            candidates.push(
                PathBuf::from(local_app_data)
                    .join("Programs")
                    .join("NeMoSpeech")
                    .join("bin"),
            );
        }
        if let Ok(executable) = env::current_exe() {
            if let Some(parent) = executable.parent() {
                candidates.push(parent.join("resources").join("NeMoSpeech").join("bin"));
                candidates.push(parent.join("NeMoSpeech").join("bin"));
                candidates.push(parent.to_path_buf());
            }
        }
        if let Some(path) = env::var_os("PATH") {
            candidates.extend(env::split_paths(&path));
        }
        candidates.sort();
        candidates.dedup();
        candidates
    }

    fn add_sdk_root(candidates: &mut Vec<PathBuf>, root: PathBuf) {
        candidates.push(root.clone());
        candidates.push(root.join("bin"));
    }

    fn add_search_directories(directory: &Path) -> Vec<DirectoryCookie> {
        let mut paths = vec![directory.to_path_buf()];
        if let Some(parent) = directory.parent() {
            paths.push(parent.join("lib"));
        }
        paths
            .into_iter()
            .filter(|path| path.is_dir())
            .filter_map(|path| {
                let wide: Vec<u16> = path
                    .as_os_str()
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                let cookie = unsafe { AddDllDirectory(wide.as_ptr()) };
                (!cookie.is_null()).then_some(cookie)
            })
            .collect()
    }

    pub struct Runtime {
        api: Arc<Api>,
    }

    impl Runtime {
        pub fn load() -> Result<Self, String> {
            Ok(Self { api: Api::load()? })
        }

        pub fn create_recognizer(&self, model_path: &Path) -> Result<Recognizer, String> {
            let model_path = model_path
                .to_str()
                .ok_or_else(|| "The Nemotron model path is not valid UTF-8.".to_owned())?;
            let model_path = CString::new(model_path)
                .map_err(|_| "The Nemotron model path contains a null character.".to_owned())?;
            let backend = BackendConfig {
                size: mem::size_of::<BackendConfig>(),
                gpu: -1,
            };
            let model = ModelConfig {
                size: mem::size_of::<ModelConfig>(),
                path: model_path.as_ptr(),
                name: ptr::null(),
            };
            let streaming = StreamingConfig {
                size: mem::size_of::<StreamingConfig>(),
                chunk_size: 0.16,
                ctc_left_padding: 1.92,
                ctc_right_padding: 1.92,
                // NVIDIA's Nemotron 3.5 streaming example uses [56, 13],
                // which corresponds to a 1.12 s RNNT chunk.
                rnnt_right_context: 13,
            };
            let config = RecognizerConfig {
                size: mem::size_of::<RecognizerConfig>(),
                backend: &backend,
                model: &model,
                streaming: &streaming as *const StreamingConfig as *const c_void,
                decoder: ptr::null(),
                vad: ptr::null(),
                // NVIDIA's SDK defaults endpointing off. This avoids token-silence
                // false endpoints on the cache-aware RNNT streaming path.
                endpointing: ptr::null(),
                postproc: ptr::null(),
                diar: ptr::null(),
            };
            let mut handle = ptr::null_mut();
            let status = unsafe { (self.api.create)(&config, &mut handle) };
            if status != 0 || handle.is_null() {
                return Err(self.api.error("recognizer initialization", status));
            }
            Ok(Recognizer {
                api: self.api.clone(),
                handle,
            })
        }
    }

    pub struct Recognizer {
        api: Arc<Api>,
        handle: *mut c_void,
    }

    impl Recognizer {
        pub fn create_stream(&self) -> Result<Stream, String> {
            static ENGLISH: &[u8] = b"en-US\0";
            let options = RecognitionOptions {
                size: mem::size_of::<RecognitionOptions>(),
                request_id: ptr::null(),
                language_code: ENGLISH.as_ptr().cast(),
                interim_results: true,
                enable_word_time_offsets: false,
                enable_automatic_punctuation: true,
                verbatim_transcripts: false,
                profanity_filter: false,
                stop_history_eou_ms: 0,
                speech_contexts: ptr::null(),
                speech_context_count: 0,
                max_alternatives: 1,
                enable_speaker_diarization: false,
                max_speaker_count: 0,
            };
            let mut handle = ptr::null_mut();
            let status =
                unsafe { (self.api.streaming_recognize)(self.handle, &options, &mut handle) };
            if status != 0 || handle.is_null() {
                return Err(self.api.error("stream creation", status));
            }
            Ok(Stream {
                api: self.api.clone(),
                handle,
                finished: false,
            })
        }
    }

    impl Drop for Recognizer {
        fn drop(&mut self) {
            if !self.handle.is_null() {
                unsafe { (self.api.destroy)(self.handle) };
                self.handle = ptr::null_mut();
            }
        }
    }

    #[derive(Debug)]
    pub struct ResultUpdate {
        pub text: String,
        pub is_final: bool,
    }

    pub struct Stream {
        api: Arc<Api>,
        handle: *mut c_void,
        finished: bool,
    }

    impl Stream {
        pub fn push(
            &mut self,
            samples: &[f32],
            sample_rate: i32,
        ) -> Result<Vec<ResultUpdate>, String> {
            if self.finished {
                return Err("The NeMo recognition stream has already ended.".into());
            }
            if samples.is_empty() {
                return Ok(Vec::new());
            }
            if !(8_000..=96_000).contains(&sample_rate) {
                return Err(format!(
                    "NeMo-Speech.cpp accepts input sample rates from 8,000 to 96,000 Hz; received {sample_rate} Hz."
                ));
            }
            let status = unsafe {
                (self.api.stream_push)(self.handle, samples.as_ptr(), samples.len(), sample_rate)
            };
            if status != 0 {
                return Err(self.api.error("audio push", status));
            }
            self.drain()
        }

        pub fn finish(&mut self) -> Result<Vec<ResultUpdate>, String> {
            if self.finished {
                return Ok(Vec::new());
            }
            let status = unsafe { (self.api.stream_finish)(self.handle) };
            if status != 0 {
                return Err(self.api.error("stream finalization", status));
            }
            let updates = self.drain()?;
            self.close();
            self.finished = true;
            Ok(updates)
        }

        fn drain(&mut self) -> Result<Vec<ResultUpdate>, String> {
            let mut updates = Vec::new();
            loop {
                let mut result = ptr::null_mut();
                let status = unsafe { (self.api.stream_next)(self.handle, &mut result) };
                if status != 0 {
                    if !result.is_null() {
                        unsafe { (self.api.result_destroy)(result) };
                    }
                    return Err(self.api.error("stream decode", status));
                }
                if result.is_null() {
                    break;
                }
                let is_final = unsafe { (self.api.result_is_final)(result) };
                let text = unsafe {
                    let transcript = (self.api.result_transcript)(result, 0);
                    if transcript.is_null() {
                        String::new()
                    } else {
                        CStr::from_ptr(transcript)
                            .to_string_lossy()
                            .trim()
                            .to_owned()
                    }
                };
                unsafe { (self.api.result_destroy)(result) };
                updates.push(ResultUpdate { text, is_final });
            }
            Ok(updates)
        }

        fn close(&mut self) {
            if !self.handle.is_null() {
                unsafe { (self.api.stream_close)(self.handle) };
                self.handle = ptr::null_mut();
            }
        }
    }

    impl Drop for Stream {
        fn drop(&mut self) {
            self.close();
        }
    }

    pub use Recognizer as PublicRecognizer;
    pub use ResultUpdate as PublicResultUpdate;
    pub use Runtime as PublicRuntime;
    pub use Stream as PublicStream;
}

#[cfg(not(windows))]
mod platform {
    use std::path::Path;

    pub struct Runtime;
    pub struct Recognizer;
    pub struct Stream;
    pub struct ResultUpdate {
        pub text: String,
        pub is_final: bool,
    }

    impl Runtime {
        pub fn load() -> Result<Self, String> {
            Err("The NeMo-Speech.cpp adapter currently supports Windows builds only.".into())
        }

        pub fn create_recognizer(&self, _model_path: &Path) -> Result<Recognizer, String> {
            unreachable!()
        }
    }

    impl Recognizer {
        pub fn create_stream(&self) -> Result<Stream, String> {
            unreachable!()
        }
    }

    impl Stream {
        pub fn push(
            &mut self,
            _samples: &[f32],
            _sample_rate: i32,
        ) -> Result<Vec<ResultUpdate>, String> {
            unreachable!()
        }

        pub fn finish(&mut self) -> Result<Vec<ResultUpdate>, String> {
            unreachable!()
        }
    }

    pub use Recognizer as PublicRecognizer;
    pub use ResultUpdate as PublicResultUpdate;
    pub use Runtime as PublicRuntime;
    pub use Stream as PublicStream;
}

pub use platform::{
    PublicRecognizer as Recognizer, PublicResultUpdate as ResultUpdate, PublicRuntime as Runtime,
    PublicStream as Stream,
};
