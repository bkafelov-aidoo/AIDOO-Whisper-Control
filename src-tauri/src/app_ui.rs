use super::*;

pub(super) fn recording_snapshot(state: &AppState) -> RecordingSnapshot {
    let state_name = state
        .recording_status
        .lock()
        .map(|value| value.clone())
        .unwrap_or_else(|_| "error".into());
    let progress = state
        .recording_progress
        .lock()
        .map(|value| value.clone())
        .unwrap_or_default();
    let elapsed_seconds = state
        .recording_started_at
        .lock()
        .ok()
        .and_then(|value| value.as_ref().map(std::time::Instant::elapsed))
        .map(|value| value.as_secs_f64())
        .unwrap_or(0.0);
    let error = state
        .last_recording_error
        .lock()
        .ok()
        .and_then(|value| value.clone());
    let trigger = state
        .recording_trigger
        .lock()
        .ok()
        .and_then(|value| value.clone());
    RecordingSnapshot {
        state: state_name,
        progress,
        elapsed_seconds,
        error,
        trigger,
    }
}

fn emit_snapshot(app: &AppHandle) {
    let _ = app.emit(
        "recording:snapshot",
        recording_snapshot(&app.state::<AppState>()),
    );
}

pub(super) fn uses_english_ui(app: &AppHandle) -> bool {
    let preference = app
        .state::<AppState>()
        .settings
        .lock()
        .map(|settings| settings.ui_language.clone())
        .unwrap_or_else(|_| "auto".into());
    preference == "en"
        || (preference == "auto"
            && !sys_locale::get_locale()
                .unwrap_or_default()
                .to_lowercase()
                .starts_with("bg"))
}

fn status_label(state: &str, english: bool) -> &'static str {
    match (english, state) {
        (true, "starting") => "Status: starting the microphone",
        (true, "recording") => "Status: recording · release the shortcut to finish",
        (true, "transcribing") => "Status: transcribing",
        (true, "done") => "Status: text is ready",
        (true, "error") => "Status: error",
        (true, "recovery") => "Status: action required",
        (true, "setup") => "Status: finish setup",
        (true, "permission") => "Status: permission required",
        (true, "wake-listening") => "Status: listening for “Hey, AIDOO”",
        (true, "wake-error") => "Status: voice activation needs attention",
        (true, "live") => "Status: AIDOO voice conversation is active",
        (true, _) => "Status: ready for dictation",
        (false, "starting") => "Състояние: стартирам микрофона",
        (false, "recording") => "Състояние: записвам · отпуснете shortcut-а за край",
        (false, "transcribing") => "Състояние: транскрибирам",
        (false, "done") => "Състояние: текстът е готов",
        (false, "error") => "Състояние: грешка",
        (false, "recovery") => "Състояние: нужно е действие",
        (false, "setup") => "Състояние: довършете настройката",
        (false, "permission") => "Състояние: нужно е разрешение",
        (false, "wake-listening") => "Състояние: слушам за „Hey, AIDOO“",
        (false, "wake-error") => "Състояние: проблем с гласовото активиране",
        (false, "live") => "Състояние: активен гласов разговор с AIDOO",
        (false, _) => "Състояние: готов за диктовка",
    }
}

pub(super) fn progress_status_label(stage: &str, english: bool) -> Option<&'static str> {
    match (english, stage) {
        (true, "preparing_audio") => Some("Status: preparing audio"),
        (true, "starting_microphone") => Some("Status: starting the microphone"),
        (true, "compressing_audio") => Some("Status: compressing to FLAC"),
        (true, "uploading_audio") => Some("Status: uploading audio"),
        (true, "openai_transcribing") => Some("Status: OpenAI is transcribing"),
        (true, "text_ready") => Some("Status: text is ready"),
        (true, "finishing_locally") => Some("Status: finishing locally"),
        (false, "preparing_audio") => Some("Състояние: подготвям аудиото"),
        (false, "starting_microphone") => Some("Състояние: стартирам микрофона"),
        (false, "compressing_audio") => Some("Състояние: компресирам в FLAC"),
        (false, "uploading_audio") => Some("Състояние: изпращам аудиото"),
        (false, "openai_transcribing") => Some("Състояние: OpenAI транскрибира"),
        (false, "text_ready") => Some("Състояние: текстът е готов"),
        (false, "finishing_locally") => Some("Състояние: завършвам локално"),
        _ => None,
    }
}

fn compact_error(error: &str) -> String {
    let normalized = error.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= 150 {
        normalized
    } else {
        format!("{}…", normalized.chars().take(149).collect::<String>())
    }
}

pub(super) fn localized_native_error(error: &str, english: bool) -> String {
    if !english {
        return error.into();
    }
    let exact = match error {
        "Ключът трябва да започва с sk-." => Some("The key must start with sk-."),
        "Не е намерен микрофон." => Some("No microphone was found."),
        "Не е намерен микрофон. Свържете устройство и опитайте отново." => {
            Some("No microphone was found. Connect an input device and try again.")
        }
        "Вече има активен запис." => Some("A recording is already active."),
        "Транскрипцията вече е стартирана." => Some("Transcription has already started."),
        "Няма активен запис." => Some("There is no active recording."),
        "Не беше разпозната реч." => Some("No speech was detected."),
        "Няма неуспешен запис за повторен опит." => {
            Some("There is no failed recording to retry.")
        }
        "Запазеният неуспешен аудио файл не е намерен." => {
            Some("The saved failed audio recording could not be found.")
        }
        "Текстът не можа да бъде поставен. Копиран е в клипборда." => {
            Some("The text could not be pasted. It remains copied to the clipboard.")
        }
        "Аудио файлът е по-голям от лимита на OpenAI от 25 MB. Направете по-кратък запис." => {
            Some("The audio file exceeds OpenAI's 25 MB limit. Make a shorter recording.")
        }
        "Достигнат е максималният запис от 5 минути. Спирам и транскрибирам." => {
            Some("The 5-minute recording limit was reached. Stopping and transcribing.")
        }
        "Има запазен неуспешен запис. Изберете „Опитай отново“ или „Изтрий“, преди да започнете нова диктовка." => {
            Some("A failed recording is saved. Choose “Try again” or “Delete” before starting a new dictation.")
        }
        "Има запазен запис за възстановяване. Завършете го или го изтрийте, преди да започнете нова диктовка." => {
            Some("A recovery item is saved. Finish or delete it before starting a new dictation.")
        }
        "Има запазен запис за възстановяване. Завършете го или го изтрийте, преди да започнете нова транскрипция." => {
            Some("A recovery item is saved. Finish or delete it before starting another transcription.")
        }
        "Транскрипцията е готова, но старият recovery запис не можа да бъде изчистен. Изберете „Изтрий“; нов опит може да доведе до повторно API таксуване." => {
            Some("The transcription succeeded, but the old recovery item could not be cleared. Choose Delete; another retry may create another API charge.")
        }
        "Възстановен е неуспешен запис след прекъсване. Можете да опитате отново." => {
            Some("A failed recording was recovered after an interruption. You can try again.")
        }
        "Възстановен е запис след прекъсване. Не може да бъде изпратен повторно автоматично, за да се избегне повторно API таксуване." => {
            Some("A recording was recovered after an interruption. It cannot be sent again automatically because that could create another API charge.")
        }
        "Завършете или отменете избора на shortcut, преди да започнете диктовка." => {
            Some("Finish or cancel shortcut selection before starting dictation.")
        }
        "Изчакайте текущата операция да приключи." => {
            Some("Wait for the current operation to finish.")
        }
        "Записът е прекалено кратък. Задръжте shortcut-а и говорете поне половин секунда." => {
            Some("The recording is too short. Hold the shortcut and speak for at least half a second.")
        }
        "Записът е прекалено кратък. Задръжте клавиша и говорете." => {
            Some("The recording is too short. Hold the key and speak.")
        }
        "Завършете началната настройка, преди да използвате диктовката." => {
            Some("Finish the initial setup before using dictation.")
        }
        "Разрешете Accessibility, за да работят shortcut-ът и автоматичното поставяне." => {
            Some("Grant Accessibility permission so the shortcut and automatic paste can work.")
        }
        "Няма достъпен OpenAI API ключ. Отворете настройките и го добавете." => {
            Some("No OpenAI API key is available. Open Settings and add one.")
        }
        "Добавете и проверете OpenAI API ключ." => {
            Some("Add and verify an OpenAI API key.")
        }
        "Настройките са заключени." => {
            Some("Settings are temporarily unavailable. Try again.")
        }
        "Историята е заключена." => {
            Some("History is temporarily unavailable. Try again.")
        }
        "Recovery състоянието е заключено." => {
            Some("Recovery is temporarily unavailable. Try again.")
        }
        "Recovery аудио файлът не е валиден." => Some("The recovery audio file is invalid."),
        "Аудио услугата не работи." => {
            Some("The audio service is unavailable. Restart the app and try again.")
        }
        "Аудио услугата не отговори." => {
            Some("The audio service did not respond. Restart the app and try again.")
        }
        "Аудио файлът е заключен." => {
            Some("The audio file is temporarily unavailable. Try again.")
        }
        "Гласовото активиране не работи." => {
            Some("Voice activation is unavailable. Restart the app and try again.")
        }
        "Гласовото активиране не отговори." => {
            Some("Voice activation did not respond. Restart the app and try again.")
        }
        "Не е намерен микрофон за гласово активиране." => {
            Some("No microphone was found for voice activation.")
        }
        "Липсва Accessibility разрешение за автоматично поставяне. Натиснете „Разреши Accessibility“ в Aidoo; разпознатият текст е запазен в Историята и clipboard." => {
            Some("Accessibility permission for automatic paste is missing. Grant Accessibility permission in AIDOO; the recognized text remains in History and the clipboard.")
        }
        _ => None,
    };
    if let Some(translated) = exact {
        return translated.into();
    }
    let prefixes = [
        ("Няма връзка с OpenAI:", "Could not connect to OpenAI:"),
        (
            "OpenAI връзката не можа да бъде подготвена:",
            "The OpenAI connection could not be prepared:",
        ),
        ("API ключът не беше приет:", "The API key was not accepted:"),
        ("Транскрипцията не успя:", "Transcription failed:"),
        (
            "OpenAI върна невалиден отговор:",
            "OpenAI returned an invalid response:",
        ),
        ("Не е намерен микрофон.", "No microphone was found."),
        (
            "Нито един микрофон не можа да стартира.",
            "No microphone could be started.",
        ),
        (
            "Избраният микрофон не е наличен. Използвам",
            "The selected microphone is unavailable. Using",
        ),
        ("Микрофонът", "Microphone"),
        (
            "Моделът за „Hey, AIDOO“ не е намерен:",
            "The “Hey, AIDOO” model was not found:",
        ),
        (
            "Wake-word моделът не може да се зареди:",
            "The wake-word model could not be loaded:",
        ),
        (
            "Микрофонът за гласово активиране прекъсна:",
            "The voice-activation microphone disconnected:",
        ),
        (
            "Гласовото активиране не можа да стартира.",
            "Voice activation could not start.",
        ),
        (
            "Пътят до wake-word модела не е достъпен:",
            "The wake-word model path is unavailable:",
        ),
        (
            "Папката не може да бъде създадена:",
            "The folder could not be created:",
        ),
        (
            "Папката не може да бъде използвана:",
            "The folder could not be used:",
        ),
        (
            "Частната папка на приложението не е валидна:",
            "The app's private data folder is invalid:",
        ),
        ("Неподдържан аудио формат:", "Unsupported audio format:"),
        (
            "Записът не можа да се запише на диска:",
            "The recording could not be written to disk:",
        ),
        ("Невалиден WAV файл:", "Invalid WAV file:"),
        (
            "FLAC поддържа mono/stereo, а записът има",
            "FLAC supports mono/stereo, but the recording has",
        ),
        (
            "FLAC процесът беше прекъснат:",
            "The FLAC process was interrupted:",
        ),
        (
            "FLAC файлът не може да бъде запазен:",
            "The FLAC file could not be saved:",
        ),
        (
            "TXT файлът не може да бъде запазен:",
            "The TXT file could not be saved:",
        ),
        (
            "Историята не можа да бъде запазена:",
            "History could not be saved:",
        ),
        (
            "Recovery аудиото не можа да бъде изтрито:",
            "The recovery audio could not be deleted:",
        ),
        (
            "Recovery състоянието не можа да бъде запазено:",
            "The recovery state could not be saved:",
        ),
        (
            "Неуспешният запис не можа да бъде запазен:",
            "The failed recording could not be retained:",
        ),
        (
            "Завършеният запис не можа да бъде запазен:",
            "The completed recording could not be retained:",
        ),
        (
            "Recovery копието не можа да бъде запазено:",
            "The recovery copy could not be retained:",
        ),
        (
            "Recovery аудио файлът не може да бъде защитен:",
            "The recovery audio file could not be protected:",
        ),
        (
            "Recovery защитата не можа да бъде обновена:",
            "The recovery retry protection could not be updated:",
        ),
        (
            "Текстът е готов, но клипбордът не е достъпен:",
            "The text is ready, but the clipboard is unavailable:",
        ),
        (
            "Транскрипцията е завършена и текстът остава в клипборда, но",
            "The transcription is complete and the text remains in the clipboard, but",
        ),
    ];
    let mut translated = error.to_string();
    for (source, target) in prefixes {
        if let Some(remainder) = error.strip_prefix(source) {
            translated = format!("{target}{remainder}");
            break;
        }
    }
    translated
        .replace(
            "невалиден или изтрит API ключ.",
            "invalid or deleted API key.",
        )
        .replace(
            "няма наличен API баланс или е достигнат лимитът.",
            "no API balance is available or the limit has been reached.",
        )
        .replace("не е наличен.", "is unavailable.")
        .replace("канала.", "channels.")
        .replace(
            "папката не може да бъде създадена:",
            "the folder could not be created:",
        )
        .replace(
            "FLAC файлът не може да бъде запазен:",
            "the FLAC file could not be saved:",
        )
        .replace(
            "TXT файлът не може да бъде запазен:",
            "the TXT file could not be saved:",
        )
        .replace(
            "историята е временно недостъпна.",
            "history is temporarily unavailable.",
        )
        .replace(
            "историята не можа да бъде запазена:",
            "history could not be saved:",
        )
        .replace(
            "Създадените локални файлове не са изтрити.",
            "Created local files were not deleted.",
        )
        .replace(
            "отговорът надвишава безопасния лимит.",
            "the response exceeds the safe limit.",
        )
        .replace(
            "Recovery аудио файлът не може да бъде защитен:",
            "The recovery audio file could not be protected:",
        )
        .replace(
            "Recovery защитата не можа да бъде обновена:",
            "The recovery retry protection could not be updated:",
        )
}

pub(super) fn build_tray_menu(app: &AppHandle, current: &str) -> tauri::Result<Menu<tauri::Wry>> {
    let english = uses_english_ui(app);
    let progress_stage = app
        .state::<AppState>()
        .recording_progress
        .lock()
        .map(|progress| progress.stage.clone())
        .unwrap_or_default();
    let mut status_text = if matches!(current, "starting" | "transcribing") {
        progress_status_label(&progress_stage, english)
            .unwrap_or_else(|| status_label(current, english))
    } else {
        status_label(current, english)
    }
    .to_string();
    let voice_recording = app
        .state::<AppState>()
        .recording_trigger
        .lock()
        .map(|trigger| trigger.as_deref() == Some("voice"))
        .unwrap_or(false);
    if current == "recording" && voice_recording {
        status_text = if english {
            "Status: recording · use Stop or pause when done".into()
        } else {
            "Състояние: записвам · натиснете Стоп или направете пауза".into()
        };
    }
    let operation_active = app
        .state::<AppState>()
        .operation_active
        .load(Ordering::Acquire)
        || matches!(current, "starting" | "recording" | "transcribing");
    let status = MenuItem::with_id(app, "status", status_text, false, None::<&str>)?;
    let show = MenuItem::with_id(
        app,
        "show",
        if english {
            "Open AIDOO Whisper Lite"
        } else {
            "Отвори AIDOO Whisper Lite"
        },
        true,
        None::<&str>,
    )?;
    let live_session_active = app
        .state::<AppState>()
        .live_session_active
        .load(Ordering::Acquire);
    let stop = MenuItem::with_id(
        app,
        "stop",
        if english && live_session_active {
            "End AIDOO conversation"
        } else if !english && live_session_active {
            "Приключи разговора с AIDOO"
        } else if english {
            "Stop and transcribe"
        } else {
            "Спри и транскрибирай"
        },
        live_session_active || matches!(current, "starting" | "recording"),
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(
        app,
        "settings",
        if english {
            "Settings"
        } else {
            "Настройки"
        },
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(
        app,
        "quit",
        if english { "Quit" } else { "Изход" },
        !operation_active,
        None::<&str>,
    )?;
    let state = app.state::<AppState>();
    let error = state
        .last_recording_error
        .lock()
        .ok()
        .and_then(|value| value.clone())
        .or_else(|| {
            state
                .wake_word_error
                .lock()
                .ok()
                .and_then(|value| value.clone())
        });
    if let Some(error) = error {
        let error = localized_native_error(&error, english);
        let error_item = MenuItem::with_id(
            app,
            "last-error",
            format!(
                "{}: {}",
                if english {
                    "Last error"
                } else {
                    "Последна грешка"
                },
                compact_error(&error)
            ),
            false,
            None::<&str>,
        )?;
        let copy_error = MenuItem::with_id(
            app,
            "copy-error",
            if english {
                "Copy error"
            } else {
                "Копирай грешката"
            },
            true,
            None::<&str>,
        )?;
        Menu::with_items(
            app,
            &[
                &status,
                &error_item,
                &copy_error,
                &show,
                &settings,
                &stop,
                &quit,
            ],
        )
    } else {
        Menu::with_items(app, &[&status, &show, &settings, &stop, &quit])
    }
}

pub(super) fn build_application_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    #[cfg(target_os = "macos")]
    {
        let app_name = app.package_info().name.clone();
        let about_metadata = AboutMetadata {
            name: Some(app_name.clone()),
            version: Some(app.package_info().version.to_string()),
            copyright: app.config().bundle.copyright.clone(),
            authors: app
                .config()
                .bundle
                .publisher
                .clone()
                .map(|value| vec![value]),
            ..Default::default()
        };
        let english = uses_english_ui(app);
        let quit = MenuItem::with_id(
            app,
            APP_QUIT_MENU_ID,
            if english {
                format!("Quit {app_name}")
            } else {
                format!("Изход от {app_name}")
            },
            true,
            Some("CmdOrCtrl+Q"),
        )?;
        let app_menu = Submenu::with_id_and_items(
            app,
            APP_MENU_ID,
            app_name,
            true,
            &[
                &PredefinedMenuItem::about(app, None, Some(about_metadata))?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::services(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::hide(app, None)?,
                &PredefinedMenuItem::hide_others(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &quit,
            ],
        )?;
        let file_menu = Submenu::with_items(
            app,
            "File",
            true,
            &[&PredefinedMenuItem::close_window(app, None)?],
        )?;
        let edit_menu = Submenu::with_items(
            app,
            "Edit",
            true,
            &[
                &PredefinedMenuItem::undo(app, None)?,
                &PredefinedMenuItem::redo(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::cut(app, None)?,
                &PredefinedMenuItem::copy(app, None)?,
                &PredefinedMenuItem::paste(app, None)?,
                &PredefinedMenuItem::select_all(app, None)?,
            ],
        )?;
        let view_menu = Submenu::with_items(
            app,
            "View",
            true,
            &[&PredefinedMenuItem::fullscreen(app, None)?],
        )?;
        let window_menu = Submenu::with_id_and_items(
            app,
            WINDOW_SUBMENU_ID,
            "Window",
            true,
            &[
                &PredefinedMenuItem::minimize(app, None)?,
                &PredefinedMenuItem::maximize(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::close_window(app, None)?,
            ],
        )?;
        let help_menu = Submenu::with_id_and_items(app, HELP_SUBMENU_ID, "Help", true, &[])?;

        Menu::with_items(
            app,
            &[
                &app_menu,
                &file_menu,
                &edit_menu,
                &view_menu,
                &window_menu,
                &help_menu,
            ],
        )
    }

    #[cfg(not(target_os = "macos"))]
    Menu::default(app)
}

pub(super) fn operation_allows_quit(operation_active: bool) -> bool {
    !operation_active
}

pub(super) fn request_app_quit(app: &AppHandle) {
    let operation_active = app
        .state::<AppState>()
        .operation_active
        .load(Ordering::Acquire);
    if operation_allows_quit(operation_active) {
        app.exit(0);
        return;
    }

    let message = if uses_english_ui(app) {
        "Wait for the current operation to finish before quitting."
    } else {
        "Изчакайте текущата операция да приключи, преди да затворите приложението."
    };
    let _ = app.emit("toast", message);
}

fn refresh_application_menu(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let operation_active = app
            .state::<AppState>()
            .operation_active
            .load(Ordering::Acquire);
        let Some(menu) = app.menu() else {
            return;
        };
        let Some(app_menu) = menu
            .get(APP_MENU_ID)
            .and_then(|item| item.as_submenu().cloned())
        else {
            return;
        };
        let Some(quit) = app_menu
            .get(APP_QUIT_MENU_ID)
            .and_then(|item| item.as_menuitem().cloned())
        else {
            return;
        };
        let app_name = app.package_info().name.clone();
        let label = if uses_english_ui(app) {
            format!("Quit {app_name}")
        } else {
            format!("Изход от {app_name}")
        };
        let _ = quit.set_text(label);
        let _ = quit.set_enabled(operation_allows_quit(operation_active));
    }
}

fn update_tray_menu(app: &AppHandle, current: &str) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let english = uses_english_ui(app);
        let tooltip = tray_tooltip(current, english);
        let _ = tray.set_tooltip(Some(tooltip));
        if let Ok(menu) = build_tray_menu(app, current) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

pub(super) fn tray_tooltip(current: &str, english: bool) -> &'static str {
    match (english, current) {
        (true, "starting") => "AIDOO Whisper Lite — starting microphone",
        (true, "recording") => "AIDOO Whisper Lite — recording",
        (true, "transcribing") => "AIDOO Whisper Lite — transcribing",
        (true, "done") => "AIDOO Whisper Lite — transcription ready",
        (true, "error") => "AIDOO Whisper Lite — error",
        (true, "recovery") => "AIDOO Whisper Lite — action required",
        (true, "setup") => "AIDOO Whisper Lite — finish setup",
        (true, "permission") => "AIDOO Whisper Lite — permission required",
        (true, "wake-listening") => "AIDOO Whisper Lite — listening for Hey, AIDOO",
        (true, "wake-error") => "AIDOO Whisper Lite — voice activation needs attention",
        (true, "live") => "AIDOO Whisper Lite — voice conversation active",
        (true, _) => "AIDOO Whisper Lite — ready",
        (false, "starting") => "AIDOO Whisper Lite — стартирам микрофона",
        (false, "recording") => "AIDOO Whisper Lite — записвам",
        (false, "transcribing") => "AIDOO Whisper Lite — транскрибирам",
        (false, "done") => "AIDOO Whisper Lite — транскрипцията е готова",
        (false, "error") => "AIDOO Whisper Lite — грешка",
        (false, "recovery") => "AIDOO Whisper Lite — нужно е действие",
        (false, "setup") => "AIDOO Whisper Lite — довършете настройката",
        (false, "permission") => "AIDOO Whisper Lite — нужно е разрешение",
        (false, "wake-listening") => "AIDOO Whisper Lite — слушам за Hey, AIDOO",
        (false, "wake-error") => "AIDOO Whisper Lite — проблем с гласовото активиране",
        (false, "live") => "AIDOO Whisper Lite — активен гласов разговор",
        (false, _) => "AIDOO Whisper Lite — готов",
    }
}

pub(super) fn refresh_tray_menu(app: &AppHandle) {
    let granted = accessibility_granted();
    let state = app.state::<AppState>();
    let current = state
        .recording_status
        .lock()
        .map(|value| value.clone())
        .unwrap_or_else(|_| "idle".into());
    let has_recovery = state
        .failed_recording
        .lock()
        .map(|recording| recording.is_some())
        .unwrap_or(true);
    let setup_ready = if matches!(current.as_str(), "idle") {
        tray_setup_ready(&state)
    } else {
        true
    };
    let mut tray_state = resolved_tray_state(&current, granted, setup_ready, has_recovery);
    if state.live_session_active.load(Ordering::Acquire) {
        tray_state = "live";
    } else if tray_state == "idle" && state.wake_word_listening.load(Ordering::Acquire) {
        tray_state = "wake-listening";
    } else if tray_state == "idle"
        && state
            .wake_word_error
            .lock()
            .map(|error| error.is_some())
            .unwrap_or(true)
    {
        tray_state = "wake-error";
    }
    update_tray_menu(app, tray_state);
    refresh_application_menu(app);
}

pub(super) fn tray_setup_ready(state: &AppState) -> bool {
    let onboarding_complete = state
        .settings
        .lock()
        .map(|settings| settings.onboarding_complete)
        .unwrap_or(false);
    let has_api_key = state
        .api_key
        .lock()
        .map(|api_key| api_key.is_some())
        .unwrap_or(false);
    onboarding_complete && has_api_key && !audio::microphone_names().is_empty()
}

pub(super) fn resolved_tray_state(
    current: &str,
    accessibility_granted: bool,
    setup_ready: bool,
    has_recovery: bool,
) -> &str {
    if matches!(current, "starting" | "recording" | "transcribing") {
        current
    } else if has_recovery {
        "recovery"
    } else if matches!(current, "done" | "error") {
        current
    } else if !setup_ready {
        "setup"
    } else if accessibility_granted {
        "idle"
    } else {
        "permission"
    }
}

pub(super) fn overlay_visible_for_state(state: &str) -> bool {
    matches!(
        state,
        "starting" | "recording" | "transcribing" | "done" | "error"
    )
}

pub(super) fn overlay_accepts_pointer_input(state: &str) -> bool {
    matches!(state, "starting" | "recording")
}

pub(super) fn set_recording_state(app: &AppHandle, next: &str) {
    let state = app.state::<AppState>();
    let mut previous = None;
    if let Ok(mut current) = state.recording_status.lock() {
        previous = Some(current.clone());
        *current = next.into();
    }
    if let Some(sound) = previous
        .as_deref()
        .and_then(|previous| feedback_sound::for_recording_transition(previous, next))
    {
        feedback_sound::play(app, sound);
    }
    if let Ok(mut started) = state.recording_started_at.lock() {
        match next {
            "recording" if started.is_none() => *started = Some(std::time::Instant::now()),
            "recording" => {}
            _ => *started = None,
        }
    }
    if next == "idle" {
        if let Ok(mut trigger) = state.recording_trigger.lock() {
            *trigger = None;
        }
    }
    let generation = state.status_generation.fetch_add(1, Ordering::Relaxed) + 1;
    refresh_tray_menu(app);
    let _ = app.emit("recording:state", next);
    if overlay_visible_for_state(next) {
        show_recording_overlay(app);
    } else if next == "idle" {
        if let Some(window) = app.get_webview_window("overlay") {
            let _ = window.hide();
        }
    }
    emit_snapshot(app);
    if matches!(next, "done" | "error") {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            if app
                .state::<AppState>()
                .status_generation
                .load(Ordering::Relaxed)
                == generation
            {
                set_recording_state(&app, "idle");
            }
        });
    } else if next == "idle" {
        schedule_wake_word_reconcile(app, std::time::Duration::from_millis(300));
    }
}

pub(super) fn set_progress(app: &AppHandle, percent: u8, stage: &str, determinate: bool) {
    let progress = RecordingProgress {
        percent: percent.min(100),
        stage: stage.into(),
        determinate,
    };
    let stage_changed = if let Ok(mut current) = app.state::<AppState>().recording_progress.lock() {
        let changed = current.stage != progress.stage;
        *current = progress.clone();
        changed
    } else {
        false
    };
    let _ = app.emit("recording:progress", &progress);
    emit_snapshot(app);
    let state_name = app
        .state::<AppState>()
        .recording_status
        .lock()
        .map(|value| value.clone())
        .unwrap_or_default();
    if stage_changed && overlay_visible_for_state(&state_name) {
        show_recording_overlay(app);
    }
    if stage_changed {
        refresh_tray_menu(app);
    }
}

pub(super) fn set_error(app: &AppHandle, error: &str) {
    storage::append_diagnostic(&format!("dictation error: {error}"));
    if let Ok(mut current) = app.state::<AppState>().last_recording_error.lock() {
        *current = Some(error.into());
    }
    let _ = app.emit("recording:error", error);
    set_recording_state(app, "error");
}

pub(crate) fn show_recording_overlay(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("overlay") {
        // Reassert the macOS panel behavior every time. This keeps the status visible above the
        // user's current app and when dictation starts from another Space or a fullscreen app.
        let _ = window.set_always_on_top(true);
        let _ = window.set_visible_on_all_workspaces(true);
        let _ = window.set_focusable(false);
        let state = app
            .state::<AppState>()
            .recording_status
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default();
        let _ = window.set_ignore_cursor_events(!overlay_accepts_pointer_input(&state));
        reposition_overlay_inner(app);
        let _ = window.show();
    }
    emit_snapshot(app);
}

fn reposition_overlay_inner(app: &AppHandle) {
    let Some(window) = app.get_webview_window("overlay") else {
        return;
    };
    let Ok(cursor) = app.cursor_position() else {
        return;
    };
    let Ok(Some(monitor)) = app.monitor_from_point(cursor.x, cursor.y) else {
        return;
    };
    let Ok(size) = window.outer_size() else {
        return;
    };
    let work_area = monitor.work_area();
    let x = work_area.position.x + (work_area.size.width.saturating_sub(size.width) / 2) as i32;
    let y = work_area.position.y + work_area.size.height.saturating_sub(size.height + 18) as i32;
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

#[tauri::command]
pub(super) fn reposition_overlay(app: AppHandle) {
    reposition_overlay_inner(&app);
}
