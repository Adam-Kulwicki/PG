// tests/logging.rs
use std::{
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};

use backend::config::{LogConfig, DEFAULT_LOG_DIR, DEFAULT_LOG_MAX_FILES, LOG_FILE_PREFIX};
use chrono::Utc;

/// Parsowanie zmiennych środowiskowych `LOG_TO_FILE`, `LOG_DIR` oraz `LOG_MAX_FILES`.
#[test]
fn parses_log_config_defaults_and_overrides() {
    assert_eq!(
        LogConfig::parse(None, None, None),
        LogConfig {
            to_file: false,
            dir: DEFAULT_LOG_DIR.to_string(),
            max_files: Some(DEFAULT_LOG_MAX_FILES),
        }
    );

    for truthy in ["true", "TRUE", "1", " yes "] {
        assert!(LogConfig::parse(Some(truthy), None, None).to_file);
    }
    for falsy in ["false", "0", "no", "", "   ", "maybe"] {
        assert!(!LogConfig::parse(Some(falsy), None, None).to_file);
    }

    assert_eq!(
        LogConfig::parse(Some("true"), Some("  /var/log/bankrut  "), Some("30")),
        LogConfig {
            to_file: true,
            dir: "/var/log/bankrut".to_string(),
            max_files: Some(30),
        }
    );

    assert_eq!(
        LogConfig::parse(Some("1"), Some("   "), Some("0")),
        LogConfig {
            to_file: true,
            dir: DEFAULT_LOG_DIR.to_string(),
            max_files: None,
        }
    );

    assert_eq!(
        LogConfig::parse(None, None, Some("invalid")).max_files,
        Some(DEFAULT_LOG_MAX_FILES)
    );
}

/// Appender tworzy dzienny plik `backend.log.YYYY-MM-DD` i zapisuje do niego linie logów.
#[test]
fn creates_daily_backend_log_file() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("bankrut-log-test-{unique}"));
    let cfg = LogConfig {
        to_file: true,
        dir: dir.to_string_lossy().into_owned(),
        max_files: Some(7),
    };

    {
        let mut appender = cfg
            .build_file_appender()
            .expect("powinien utworzyć appender");
        writeln!(appender, "test log entry").unwrap();
        appender.flush().unwrap();
    }

    let today = Utc::now().format("%Y-%m-%d").to_string();
    let expected_file = dir.join(format!("{LOG_FILE_PREFIX}.{today}"));
    let content = std::fs::read_to_string(&expected_file).expect("plik logu powinien istnieć");
    assert!(content.contains("test log entry"));

    std::fs::remove_dir_all(&dir).unwrap();
}
