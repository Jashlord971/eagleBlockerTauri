// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::WindowEvent;
use std::net::TcpListener;
use std::fs;
use std::path::PathBuf;
use tauri::api::path::app_data_dir;
use serde_json::{Map, Value}; 
use std::process::Command;
use std::str;
use regex::Regex;
use std::io::Write;
use std::env;
use std::sync::{Mutex, Arc, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::time::Duration;
use std::collections::HashSet;
use tauri::Manager;
use tauri::{CustomMenuItem, Menu, WindowMenuEvent};
use std::path::Path;
use serde_json::json;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use winreg::RegKey;
use std::collections::HashMap;
use std::fs::File;
use std::time::{SystemTime, UNIX_EPOCH};
use once_cell::sync::Lazy;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
#[cfg(windows)]
use windows::Win32::System::SystemInformation::GetTickCount64;

mod browser_detector;
use browser_detector::BrowserDetector;
static BROWSER_DETECTOR: Lazy<BrowserDetector> = Lazy::new(|| BrowserDetector::new());
static OVERLAY_OPEN: Lazy<std::sync::atomic::AtomicBool> = Lazy::new(|| std::sync::atomic::AtomicBool::new(false));
const CREATE_NO_WINDOW: u32 = 0x08000000;
const DELAY_TIMEOUT_KEY: &str = "delayTimeOutAtTimeOfChange";

static DNS_SAFE_CACHE: Lazy<Mutex<Option<(bool, std::time::Instant)>>> =
    Lazy::new(|| Mutex::new(None));
const DNS_SAFE_TTL: std::time::Duration = std::time::Duration::from_secs(120);

pub(crate) fn run_hidden_output(program: &str, args: &[&str]) -> Result<std::process::Output, String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.output().map_err(|e| format!("failed to spawn {}: {}", program, e))
}

static FILE_IO_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

static ACTIVE_TIMERS: Lazy<Mutex<HashMap<String, (Arc<AtomicBool>, std::thread::JoinHandle<()>, u64)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static PROTECTION_HANDLE: Lazy<Mutex<Option<thread::JoinHandle<()>>>> = Lazy::new(|| Mutex::new(None));
static PROTECTION_STOP: Lazy<Mutex<Option<Arc<AtomicBool>>>> = Lazy::new(|| Mutex::new(None));
static CURRENT_PAGE: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));
static BLOCK_DATA_CACHE: Lazy<Mutex<Option<Map<String, Value>>>> = Lazy::new(|| Mutex::new(None));
static SAVED_PREFERENCES_CACHE: Lazy<Mutex<Option<Map<String, Value>>>> = Lazy::new(|| Mutex::new(None));

const REQUIRED_ENTRIES : [&str; 7]= [
    "216.239.38.120 www.google.com",
    "216.239.38.120 google.com",
    "204.79.197.220 bing.com",
    "204.79.197.220 www.bing.com",
    "213.180.193.56 yandex.ru",
    "213.180.204.92 www.yandex.com",
    "127.0.0.1 yandex.com/images",
];

const PROTECTED_SYSTEM_APPS : [(&str, &[&str]); 3] = [
    ("Task Manager", &["taskmgr.exe"]),
    ("Task Scheduler", &["mmc.exe"]),
    ("Control Panel", &["control.exe"])
];

const TASK_NAME: &str = "Eagle Task Schedule";
const HOSTS_PATH: &str = r"C:\Windows\System32\drivers\etc\hosts";
const DELAY_SETTINGS: &str = "delayTimeOut";
const UNINSTALL_OVERLAY_DISPLAY: &str = "Uninstaller";

fn get_preferences_path(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    let mut path = app_data_dir(&app_handle.config()).ok_or("No app data dir")?;
    path.push("savedPreferences.json");
    return Ok(path);
}

fn read_preferences_file(path: &PathBuf) -> Result<Map<String, Value>, String> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
    return serde_json::from_str(&content).map_err(|e| e.to_string());
}

fn write_preferences_file(path: &PathBuf, preferences: &Map<String, Value>) -> Result<(), String> {
    let content = serde_json::to_string(preferences).map_err(|e| e.to_string())?;
    fs::write(path, content).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_preference(key: String, value: serde_json::Value, app_handle: tauri::AppHandle) -> Result<(), String> {
    println!("save_preference: adding/updating key = '{}'", &key);

    let path = get_preferences_path(&app_handle)?;
    let mut preferences = read_preferences_file(&path)?;

    match serde_json::to_string_pretty(&preferences) {
        Ok(s) => println!("Preferences before insert:\n{}", s),
        Err(_) => println!("Preferences before insert: <failed to serialize>"),
    }

    preferences.insert(key.clone(), value);

    write_preferences_file(&path, &preferences)?;

    match read_preferences_file(&path) {
        Ok(verify) => match serde_json::to_string_pretty(&verify) {
            Ok(s) => println!("Preferences after write:\n{}", s),
            Err(_) => println!("Preferences after write: <failed to serialize>"),
        },
        Err(e) => println!("Failed to read back preferences after write: {}", e),
    }

    close_confirmation_dialog(app_handle);

    Ok(())
}

fn get_app_file_path(app_handle: &tauri::AppHandle, filename: &str) -> Result<PathBuf, String> {
    let mut path = app_data_dir(&app_handle.config()).ok_or("No app data dir")?;
    if !path.exists() {
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    }
    path.push(filename);
    Ok(path)
}

fn backup_corrupted_file(path: &PathBuf, content: &str) -> Result<PathBuf, String> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let file_name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => format!("{}.corrupt.{}", n, ts),
        None => format!("corrupt.{}", ts),
    };
    if let Some(parent) = path.parent() {
        let mut corrupt_path = parent.to_path_buf();
        corrupt_path.push(file_name);
        fs::write(&corrupt_path, content).map_err(|e| format!("failed to write corrupt backup: {}", e))?;
        return Ok(corrupt_path);
    }
    Err("failed to determine parent dir for corrupt backup".into())
}

fn read_json_map(path: &PathBuf) -> Result<Map<String, Value>, String> {
    let _guard = FILE_IO_LOCK.lock().map_err(|e| e.to_string())?;

    if !path.exists() {
        println!("read_json_map: file does not exist: {}", path.display());
        return Ok(Map::new());
    }

    let content = fs::read_to_string(path).map_err(|e| format!("read_json_map read error: {}", e))?;

    match serde_json::from_str::<Map<String, Value>>(&content) {
        Ok(map) => return Ok(map),
        Err(parse_err) => {
            eprintln!("read_json_map: failed to parse {}: {}", path.display(), parse_err);
            if let (Some(first), Some(last)) = (content.find('{'), content.rfind('}')) {
                if last > first {
                    let candidate = &content[first..=last];
                    match serde_json::from_str::<Map<String, Value>>(candidate) {
                        Ok(repaired_map) => {
                            println!("read_json_map: salvage succeeded for {}, backing up original and writing repaired JSON", path.display());

                            if let Err(e) = backup_corrupted_file(path, &content) {
                                eprintln!("read_json_map: backup_corrupted_file failed: {}", e);
                            }
                            let pretty = serde_json::to_string_pretty(&repaired_map).map_err(|e| format!("salvaged to_string_pretty error: {}", e))?;
                            let mut tmp = path.clone();
                            tmp.set_extension("tmp");
                            fs::write(&tmp, pretty.as_bytes()).map_err(|e| format!("failed to write temp repaired file: {}", e))?;
                            fs::rename(&tmp, path).map_err(|e| format!("failed to move repaired file into place: {}", e))?;
                            return Ok(repaired_map);
                        }
                        Err(_) => {
                            eprintln!("read_json_map: salvage attempt failed for {}. Backing up corrupted file and returning empty prefs.", path.display());
                            let _ = backup_corrupted_file(path, &content);
                            return Ok(Map::new());
                        }
                    }
                }
            }
            eprintln!("read_json_map: no salvage possible for {}, backing up and returning empty prefs", path.display());
            let _ = backup_corrupted_file(path, &content);
            return Ok(Map::new());
        }
    }
}

fn write_json_map(path: &PathBuf, map: &Map<String, Value>) -> Result<(), String> {
    let _guard = FILE_IO_LOCK.lock().map_err(|e| e.to_string())?;

    let content = serde_json::to_string_pretty(map).map_err(|e| format!("write_json_map serialize error: {}", e))?;
    println!("write_json_map: writing to path: {}", path.display());
    println!("write_json_map: content:\n{}", content);

    let mut tmp = path.clone();
    tmp.set_extension("tmp");

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("failed to create parent dir: {}", e))?;
    }

    {
        let mut f = fs::File::create(&tmp).map_err(|e| format!("failed to create temp file: {}", e))?;
        f.write_all(content.as_bytes()).map_err(|e| format!("failed to write temp file: {}", e))?;
        f.sync_all().map_err(|e| format!("failed to sync temp file: {}", e))?;
    }

    fs::rename(&tmp, path).map_err(|e| format!("failed to rename temp to target: {}", e))?;

    if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
        match filename {
            "blockData.json" => {
                if let Ok(mut guard) = BLOCK_DATA_CACHE.lock() {
                    *guard = Some(map.clone());
                    println!("write_json_map: updated blockData cache");
                }
            }
            "savedPreferences.json" => {
                if let Ok(mut guard) = SAVED_PREFERENCES_CACHE.lock() {
                    *guard = Some(map.clone());
                    println!("write_json_map: updated savedPreferences cache");
                }
            }
            _ => {}
        }
    }

    Ok(())
}

#[tauri::command]
fn read_preference(key: String, app_handle: tauri::AppHandle) -> Result<bool, String> {
    let path = get_app_file_path(&app_handle, "savedPreferences.json")?;
    let prefs = read_json_map(&path)?;
    Ok(prefs.get(&key).and_then(|v| v.as_bool()).unwrap_or(false))
}

#[tauri::command]
fn get_delay_time_out(app_handle: tauri::AppHandle) -> Result<u64, String> {
    let path = get_app_file_path(&app_handle, "savedPreferences.json")?;
    let prefs = read_json_map(&path)?;

    let value = prefs.get(DELAY_SETTINGS)
        .and_then(|v| v.as_u64())
        .or_else(|| prefs.get(DELAY_SETTINGS).and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok()))
        .unwrap_or(180_000u64);

    Ok(value)
}

#[tauri::command]
fn get_block_data(app_handle: tauri::AppHandle) -> Result<Map<String, Value>, String> {
    let path = get_app_file_path(&app_handle, "blockData.json")?;
    read_json_map(&path)
}

#[tauri::command]
fn save_block_data(data: Map<String, Value>, app_handle: tauri::AppHandle) -> Result<(), String> {
    let path = get_app_file_path(&app_handle, "blockData.json")?;
    write_json_map(&path, &data)
}

#[tauri::command]
fn is_dns_made_safe() -> Result<bool, String> {
    is_dns_made_safe_cached()
}

fn dns_cache_set(value: bool) {
    if let Ok(mut g) = DNS_SAFE_CACHE.lock() {
        *g = Some((value, std::time::Instant::now()));
    }
}

fn is_dns_made_safe_cached() -> Result<bool, String> {
    let now = std::time::Instant::now();
    if let Ok(mut g) = DNS_SAFE_CACHE.lock() {
        if let Some((val, ts)) = g.as_ref().copied() {
            if now.duration_since(ts) < DNS_SAFE_TTL {
                return Ok(val);
            }
        }
        let interface_name = get_active_interface_name()?;
        let fresh = is_safe_dns(&interface_name)?;
        *g = Some((fresh, now));
        return Ok(fresh);
    }
    let interface_name = get_active_interface_name()?;
    is_safe_dns(&interface_name)
}

#[tauri::command]
async fn turn_on_dns(is_strict: bool, app_handle: tauri::AppHandle) -> Result<(), String> {
    let interface_name = get_active_interface_name()?;

    configure_safe_dns(&interface_name, is_strict).await.map_err(|e| {
        eprintln!("turn_on_dns: configure_safe_dns failed: {}", e);
        let elow = e.to_lowercase();
        if elow.contains("elevation canceled") || elow.contains("canceled by the user") || elow.contains("operation was canceled") {
            "elevation-canceled-by-user".to_string()
        } else {
            format!("configure_safe_dns failed: {}", e)
        }
    })?;

    dns_cache_set(true);

    save_preference(
        "enableProtectiveDNS".to_string(),
        serde_json::Value::Bool(true),
        app_handle.clone(),
    )?;

    let _ = tauri::Manager::emit_all(
        &app_handle,
        "main-config-updated",
        serde_json::json!({}),
    );

    Ok(())
}

fn get_active_interface_name() -> Result<String, String> {
    let output = run_hidden_output("netsh", &["interface", "show", "interface"]) 
        .map_err(|e| format!("Failed to execute netsh command: {}", e))?;

    if !output.status.success() {
        return Err("Failed to retrieve active interfaces".to_string());
    }

    let stdout = str::from_utf8(&output.stdout).map_err(|e| format!("Invalid UTF-8 output: {}", e))?;
    let re = Regex::new(r"\s{2,}").map_err(|e| e.to_string())?;

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.contains("Connected") && trimmed.contains("Enabled") {
            let parts: Vec<&str> = re.split(trimmed).map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
            if let Some(interface_name) = parts.last() {
                return Ok(interface_name.to_string());
            }
        }
    }

    Err("No active interface found".to_string())
}

fn is_safe_dns(interface_name: &str) -> Result<bool, String> {
    let output = run_hidden_output(
        "netsh",
        &["interface", "ipv4", "show", "dnsservers", "name=", interface_name],
    )
    .map_err(|e| format!("Failed to execute netsh command: {}", e))?;

    if !output.status.success() {
        return Err("Failed to retrieve DNS settings".to_string());
    }

    let stdout = str::from_utf8(&output.stdout).map_err(|e| format!("Invalid UTF-8 output: {}", e))?;

    let has_strict_dns = stdout.contains("185.228.168.168") && stdout.contains("185.228.169.168");
    let has_lenient_dns = stdout.contains("208.67.222.123") && stdout.contains("208.67.220.123");

    Ok(has_strict_dns || has_lenient_dns)
}

async fn run_elevated_command(cmd: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let ps = format!(
            "Start-Process -FilePath 'cmd.exe' -ArgumentList '/C','{}' -Verb RunAs -WindowStyle Hidden -Wait; exit $LASTEXITCODE",
            cmd.replace('\'', r#"'"#)
        );

        let output = run_hidden_output(
            "powershell",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &ps,
            ],
        )
        .map_err(|e| format!("failed to spawn powershell: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let code = output.status.code().unwrap_or(-1);

        if output.status.success() {
            return Ok(());
        }

        if code == 1223
            || stderr.to_lowercase().contains("canceled by the user")
            || stderr.to_lowercase().contains("operation was canceled")
        {
            println!("Action was canceled by the user");
            return Err("elevation canceled by user".into());
        }

        Err(format!(
            "elevated command failed: code={:?}, stdout={}, stderr={}",
            code, stdout, stderr
        ))
    })
    .await
    .map_err(|e| format!("spawn_blocking join error: {}", e))?
}

async fn configure_safe_dns(interface_name: &str, is_strict: bool) -> Result<(), String> {
    let (primary_dns, secondary_dns) = if is_strict {
        ("185.228.168.168", "185.228.169.168")
    } else {
        ("208.67.222.123", "208.67.220.123")
    };

    let netsh_command = format!(
        "netsh interface ipv4 set dns name=\"{}\" static {} primary && netsh interface ipv4 add dns name=\"{}\" {} index=2",
        interface_name, primary_dns, interface_name, secondary_dns
    );

    println!("Command: {}", &netsh_command);
    run_elevated_command(netsh_command).await
}

#[tauri::command]
fn is_safe_search_enabled() -> Result<bool, String> {

    match std::fs::read_to_string(HOSTS_PATH) {
        Ok(content) => {
            let all_present = REQUIRED_ENTRIES.iter().all(|entry| content.contains(entry));
            Ok(all_present)
        }
        Err(e) => {
            eprintln!("Error reading hosts file ({}): {}", HOSTS_PATH, e);
            Ok(false)
        }
    }
}

#[tauri::command]
async fn enable_safe_search(app_handle: tauri::AppHandle) -> Result<bool, String>{
    let current = match std::fs::read_to_string(HOSTS_PATH) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("add_safe_search_hosts: failed to read hosts file: {}", e);
            String::new()
        }
    };

    let mut to_add: Vec<&str> = Vec::new();
    for entry in &REQUIRED_ENTRIES {
        if !current.contains(entry) {
            to_add.push(entry);
        }
    }

    if to_add.is_empty() {
        return Ok(false);
    }

    let mut temp_path = env::temp_dir();
    temp_path.push("eagleblocker_hosts_tmp");
    let temp_path_str = temp_path.to_string_lossy().into_owned();
    
    let mut file = std::fs::File::create(&temp_path).map_err(|e| format!("failed to create temp hosts file: {}", e))?;
    file.write_all(current.as_bytes()).map_err(|e| format!("failed to write current hosts to temp file: {}", e))?;
    
    if !current.ends_with('\n') {
        file.write_all(b"\n").map_err(|e| format!("failed to write newline to temp file: {}", e))?;
    }
    
    for entry in to_add.iter() {
        file.write_all(entry.as_bytes()).map_err(|e| format!("failed to append entry to temp file: {}", e))?;
        file.write_all(b"\r\n").map_err(|e| format!("failed to append newline to temp file: {}", e))?;
    }

    let cmd = format!("move /Y \"{}\" \"{}\"", temp_path_str.replace('"', ""), HOSTS_PATH);

    run_elevated_command(cmd).await.map_err(|e| format!("elevated move failed: {}", e))?;
    
    if let Err(e) = save_preference(
        "enforceSafeSearch".to_string(),
        serde_json::Value::Bool(true),
        app_handle,
    ) {
        eprintln!("add_safe_search_hosts: failed to save preference: {}", e);
    }

    Ok(true)
}

fn get_running_process_names() -> Result<HashSet<String>, String> {
    let output = run_hidden_output("tasklist", &["/FO", "CSV", "/NH"]) 
        .map_err(|e| format!("tasklist spawn failed: {}", e))?;
    if !output.status.success() {
        return Err("tasklist failed".into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut set = HashSet::new();

    for line in stdout.lines() {
        if let Some(name_field) = line.split(',').next() {
            let name = name_field.trim().trim_matches('"').to_lowercase();
            if !name.is_empty() { 
                set.insert(name); 
            }
        }
    }

    Ok(set)
}

// Simple CSV parser for tasklist /FO CSV lines (handles quoted fields and embedded commas)
fn parse_csv_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                if in_quotes {
                    if matches!(chars.peek(), Some('"')) {
                        // escaped quote inside quoted field
                        cur.push('"');
                        let _ = chars.next();
                    } else {
                        in_quotes = false;
                    }
                } else {
                    in_quotes = true;
                }
            }
            ',' if !in_quotes => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    out.push(cur.trim().to_string());
    out
}

fn load_block_data(app_handle: &tauri::AppHandle) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    let path = get_app_file_path(app_handle, "blockData.json")?;
    read_json_map(&path)
}

fn collect_blocked_apps(map: &serde_json::Map<String, serde_json::Value>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(serde_json::Value::Array(arr)) = map.get("blockedApps") {
        for (_i, item) in arr.iter().enumerate() {
            let proc_opt = item.as_str().map(|s| s.to_string()).or_else(|| {
                item.get("processName").and_then(|v| v.as_str()).map(|s| s.to_string())
            });
            let disp_opt = item.get("displayName").and_then(|v| v.as_str()).map(|s| s.to_string())
                .or_else(|| proc_opt.clone());

            match proc_opt {
                Some(proc_name) => {
                    let display = disp_opt.unwrap_or_else(|| proc_name.clone());
                    out.push((proc_name, display));
                }
                None => {
                    println!("collect_blocked_apps: skipping entry (no processName): {}", item);
                }
            }
        }
    }
    out
}

fn process_matches_running(proc_name: &str, running: &std::collections::HashSet<String>) -> bool {
    let norm = proc_name.to_lowercase();
    let norm_exe = if norm.ends_with(".exe") { norm.clone() } else { format!("{}.exe", norm) };
    running.iter().any(|r| {
        let rnorm = r.to_lowercase();
        rnorm == norm || rnorm == norm_exe
    })
}

fn is_embedded_webview(process_name: &str) -> bool {
    let p = process_name.to_lowercase();
    p.contains("msedgewebview2") || p.contains("webview2") || p.contains("bravecrashhandler")
}

fn is_uninstall_window_title_visible() -> Result<bool, String> {
    let query = "eagleblocker Uninstall";
    let output = run_hidden_output("tasklist", &["/FO", "CSV", "/NH", "/V"])
        .map_err(|e| format!("tasklist /V spawn failed: {}", e))?;
    if !output.status.success() {
        return Ok(false);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let needle = query.to_lowercase();
    for line in stdout.lines() {
        if line.trim().is_empty() { continue; }
        let cols = parse_csv_line(line);
        if cols.is_empty() { continue; }
        let window_title = cols.last().map(|s| s.trim_matches('"')).unwrap_or("");
        if !window_title.is_empty() && window_title.to_lowercase().contains(&needle) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(windows)]
fn get_idle_millis() -> Result<u64, String> {
    unsafe {
        let mut lii = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if !GetLastInputInfo(&mut lii).as_bool() {
            return Err("GetLastInputInfo failed".into());
        }
        let now = GetTickCount64();
        let idle_ms = now.saturating_sub(lii.dwTime as u64);
        Ok(idle_ms)
    }
}

fn is_pc_idle(threshold_ms: u64) -> Result<bool, String> {
    #[cfg(windows)]
    {
        let idle = get_idle_millis()?;
        return Ok(idle >= threshold_ms);
    }
    #[cfg(not(windows))]
    {
        Ok(false)
    }
}

#[tauri::command]
fn turn_on_settings_and_app_protection(app_handle: tauri::AppHandle) -> Result<bool, String> {
    let mut guard = PROTECTION_HANDLE.lock().map_err(|e| e.to_string())?;
    if guard.is_some() {
        return Ok(true);
    }

    let stop_flag = Arc::new(AtomicBool::new(false));
    *PROTECTION_STOP.lock().map_err(|e| e.to_string())? = Some(stop_flag.clone());

    let app_clone = app_handle.clone();
    let handle = std::thread::spawn(move || {
        let interval = Duration::from_secs(4);
        let sync_interval = Duration::from_secs(90);
        let mut last_sync = std::time::Instant::now();
        loop {
            if stop_flag.load(Ordering::SeqCst) {
                break;
            }

            if is_pc_idle(2 * 60 * 1000).unwrap_or(false) {
                std::thread::sleep(interval);
                continue;
            }

            if last_sync.elapsed() >= sync_interval {
                if let Err(e) = perform_sync_recovery(&app_clone) {
                    eprintln!("protection thread: sync/recovery failed: {}", e);
                }
                last_sync = std::time::Instant::now();
            }
            
            let is_settings_protection_on = read_preferences_for_key(&app_clone, "blockSettingsSwitch").unwrap_or(false);

            if !is_settings_protection_on {
                break;
            }

            let is_dns_protection_on = read_preferences_for_key(&app_clone, "enableProtectiveDNS").unwrap_or(false);

            if is_settings_protection_on {
                let mut has_flagged = false;
                if has_flagged == false {
                    if is_uninstall_window_title_visible().unwrap_or(false) {
                        let arguments = serde_json::json!({
                            "displayName": UNINSTALL_OVERLAY_DISPLAY,
                            "processName": "uninstaller",
                            "code" : "uninstaller-window-detected"
                        });
                        let _ = show_overlay(&app_clone, arguments);
                        has_flagged = true;
                    }
                }

                if !has_flagged {
                    match get_running_process_names() {
                        Ok(running) => {
                            for (display, procs) in PROTECTED_SYSTEM_APPS.iter() {
                                if procs.iter().any(|p| running.contains(*p)) {
                                    let arguments = serde_json::json!({
                                        "displayName": display,
                                        "processName": procs[0],
                                        "code" : "protected-system-app"
                                    });

                                    let _ = show_overlay(&app_clone, arguments);
                                    has_flagged = true;
                                    break;
                                }
                            }

                            if has_flagged == false {
                                if let Ok(block_map) = load_block_data(&app_clone) {
                                    let blocked_apps = collect_blocked_apps(&block_map);
                                    for (proc_name, display_name) in blocked_apps.iter() {
                                        if process_matches_running(proc_name, &running) {
                                            let arguments = serde_json::json!({
                                                "displayName": display_name,
                                                "processName": proc_name,
                                                "code" : "blocked-app"
                                            });
                                            let _ = show_overlay(&app_clone, arguments);
                                            has_flagged = true;
                                            break;
                                        }
                                    }
                                }
                            }
                            
                            if has_flagged == false && is_dns_protection_on {
                                if BROWSER_DETECTOR.is_tor_proxy_running_enhanced() {
                                    for process_name in running.iter() {
                                        if BROWSER_DETECTOR.is_browser_application(process_name) && !is_embedded_webview(process_name) {
                                            let arguments = serde_json::json!({
                                                "displayName": process_name,
                                                "processName": process_name,
                                                "code" : "browser-with-proxy"
                                            });

                                            let _ = show_overlay(&app_clone, arguments);

                                            has_flagged = true;
                                            println!("Found a registered browser running while the tor connection is on. Flagged!");
                                            break;
                                        }
                                    }
                                }
                                if has_flagged == true {
                                    continue;
                                }

                                if running.contains(&"brave.exe".to_string())  || running.contains(&"chrome.exe".to_string()) || running.contains(&"msedge.exe".to_string()) {
                                    match detect_vpn_proxy_all_browsers() {
                                        Ok(extensions) => {
                                            if !extensions.is_empty() {
                                                let mut browsers: HashSet<String> = HashSet::new();
                                                for ext in extensions.iter() {
                                                    let is_vpn = ext.get("is_vpn").and_then(|v| v.as_bool()).unwrap_or(false);
                                                    let has_proxy = ext.get("has_proxy_permission").and_then(|v| v.as_bool()).unwrap_or(false);
                                                    if is_vpn && has_proxy {
                                                        if let Some(bname) = ext.get("browser").and_then(|v| v.as_str()) {
                                                            browsers.insert(bname.to_string());
                                                        }
                                                    }
                                                }

                                                let map_proc = |b: &str| -> Option<(&'static str, &'static str)> {
                                                    match b {
                                                        "Chrome" => Some(("Google Chrome", "chrome.exe")),
                                                        "Edge" => Some(("Microsoft Edge", "msedge.exe")),
                                                        "Brave" => Some(("Brave", "brave.exe")),
                                                        _ => None,
                                                    }
                                                };

                                                let mut blocked = false;
                                                for b in browsers {
                                                    if let Some((display, exe)) = map_proc(&b) {
                                                        if running.contains(&exe.to_lowercase()) {
                                                            let _ = show_overlay(
                                                                &app_clone,
                                                                serde_json::json!({
                                                                    "displayName": display,
                                                                    "processName": exe,
                                                                    "code": "browser-with-vpn"
                                                                }),
                                                            );
                                                            has_flagged = true;
                                                            blocked = true;
                                                            break;
                                                        }
                                                    }
                                                }

                                                if !blocked {
                                                    println!("VPN extensions present, but none of the affected browsers are currently running.");
                                                }
                                            } else {
                                                println!("✅ No VPN proxy extensions detected");
                                            }
                                        }
                                        Err(e) => {
                                            eprintln!("VPN detection thread: detection failed: {}", e);
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => eprintln!("protection thread: get_running_process_names failed: {}", e),
                    }
                }

                if has_flagged == false {
                    close_overlay_window(app_clone.clone());
                }
            }
            else{
                return;
            }

            std::thread::sleep(interval);
        }
    });

    *guard = Some(handle);
    
    let _ = create_eagle_task_schedule_simple();
    Ok(true)
}

fn extract_vpn_extension(manifest: &serde_json::Value, manifest_text: &str, manifest_path: &PathBuf, browser_name: &str) -> Option<serde_json::Value> {
    let has_proxy = manifest.get("permissions")
        .and_then(|p| p.as_array())
        .map(|a| {
            a.iter().any(|v| {
                v.as_str().map(|s| s == "proxy" || s == "webRequest").unwrap_or(false)
            })
        })
        .unwrap_or(false);

    if !has_proxy {
        return None;
    }

    let ext_id = manifest_path.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("Unknown")
        .to_string();

    let name = manifest.get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_string();

    let version = manifest.get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_string();

    let homepage = manifest.get("homepage_url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let description = manifest.get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let manifest_lower = manifest_text.to_lowercase();
    
    let is_vpn = manifest_lower.contains("veepn") 
        || manifest_lower.contains("vpn")
        || homepage.to_lowercase().contains("veepn")
        || homepage.to_lowercase().contains("vpn")
        || name.to_lowercase().contains("vpn")
        || description.to_lowercase().contains("vpn");

    if !is_vpn {
        return None;
    }

    let permissions = manifest.get("permissions")
        .and_then(|p| p.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect::<Vec<_>>())
        .unwrap_or_default();

    Some(json!({
        "id": ext_id,
        "name": name,
        "version": version,
        "homepage_url": homepage,
        "description": description,
        "browser": browser_name,
        "has_proxy_permission": has_proxy,
        "is_vpn": true,
        "permissions": permissions,
        "manifest_path": manifest_path.to_string_lossy().to_string()
    }))
}

fn run_powershell_hidden(script: &str) -> Result<std::process::Output, String> {
    let args = [
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy", "Bypass",
        "-WindowStyle", "Hidden",
        "-Command",
        script,
    ];
    run_hidden_output("powershell.exe", &args)
}

fn detect_vpn_proxy_all_browsers() -> Result<Vec<serde_json::Value>, String> {
    let ps = r#"
    $ErrorActionPreference = 'SilentlyContinue'
    $result = @()

    # Chromium-family (Brave/Chrome/Edge)
    $local = $env:LOCALAPPDATA
    $targets = @(
        @{ Name='Brave'; Path=(Join-Path $local 'BraveSoftware\Brave-Browser\User Data\Default\extensions') },
        @{ Name='Chrome'; Path=(Join-Path $local 'Google\Chrome\User Data\Default\extensions') },
        @{ Name='Edge'; Path=(Join-Path $local 'Microsoft\Edge\User Data\Default\extensions') }
    )
    foreach($t in $targets){
        if(Test-Path $t.Path){
            Get-ChildItem $t.Path -Directory -ErrorAction SilentlyContinue | ForEach-Object {
                $extId = $_.Name
                Get-ChildItem $_.FullName -Directory -ErrorAction SilentlyContinue | ForEach-Object {
                    $mf = Join-Path $_.FullName 'manifest.json'
                    if(Test-Path $mf){
                        try {
                            $j = Get-Content $mf -Raw | ConvertFrom-Json
                            $perms = @()
                            if($j.permissions){ $perms = @($j.permissions) }
                            $hasProxy = $perms -contains 'proxy' -or $perms -contains 'webRequest'
                            $text = Get-Content $mf -Raw
                            $name = $j.name
                            $desc = $j.description
                            $home = $j.homepage_url
                            $isVPN = ($text -match '(?i)vpn|veepn') -or ($name -match '(?i)vpn') -or ($desc -match '(?i)vpn') -or ($home -match '(?i)vpn')
                            if($hasProxy -and $isVPN){
                                $result += [pscustomobject]@{
                                    id = $extId
                                    name = $name
                                    version = $j.version
                                    homepage_url = $home
                                    description = $desc
                                    permissions = $perms
                                    browser = $t.Name
                                    has_proxy_permission = $hasProxy
                                    is_vpn = $true
                                    manifest_path = $mf
                                }
                            }
                        } catch {}
                    }
                }
            }
        }
    }

    # Firefox in Roaming and Local
    $paths = @(
        (Join-Path $env:APPDATA 'Mozilla\Firefox\Profiles'),
        (Join-Path $env:LOCALAPPDATA 'Mozilla\Firefox\Profiles')
    )
    foreach($p in $paths){
        if(-not (Test-Path $p)){ continue }
        # 1) Raw manifests on disk
        Get-ChildItem $p -Recurse -Filter 'manifest.json' -ErrorAction SilentlyContinue | ForEach-Object {
            try {
                $mf = $_.FullName
                $j = Get-Content $mf -Raw | ConvertFrom-Json
                $perms = @()
                if($j.permissions){ $perms = @($j.permissions) }
                $hasProxy = $perms -contains 'proxy' -or $perms -contains 'webRequest'
                $text = Get-Content $mf -Raw
                $name = $j.name
                $desc = $j.description
                $home = $j.homepage_url
                $isVPN = ($text -match '(?i)vpn|veepn') -or ($name -match '(?i)vpn') -or ($desc -match '(?i)vpn') -or ($home -match '(?i)vpn')
                if($hasProxy -and $isVPN){
                    $extId = Split-Path (Split-Path $mf -Parent) -Leaf
                    $result += [pscustomobject]@{
                        id = $extId
                        name = $name
                        version = $j.version
                        homepage_url = $home
                        description = $desc
                        permissions = $perms
                        browser = 'Firefox'
                        has_proxy_permission = $hasProxy
                        is_vpn = $true
                        manifest_path = $mf
                    }
                }
            } catch {}
        }

        # 2) Packed .xpi add-ons
        try { Add-Type -AssemblyName System.IO.Compression.FileSystem | Out-Null } catch {}
        Get-ChildItem $p -Recurse -Filter '*.xpi' -ErrorAction SilentlyContinue | ForEach-Object {
            try {
                $zip = [System.IO.Compression.ZipFile]::OpenRead($_.FullName)
                $entry = $zip.Entries | Where-Object { $_.FullName -eq 'manifest.json' -or $_.Name -eq 'manifest.json' } | Select-Object -First 1
                if($entry){
                    $sr = New-Object System.IO.StreamReader($entry.Open())
                    $text = $sr.ReadToEnd(); $sr.Close(); $zip.Dispose()
                    $j = $text | ConvertFrom-Json
                    $perms = @(); if($j.permissions){ $perms = @($j.permissions) }
                    $hasProxy = $perms -contains 'proxy' -or $perms -contains 'webRequest'
                    $name = $j.name; $desc = $j.description; $home = $j.homepage_url
                    $isVPN = ($text -match '(?i)vpn|veepn') -or ($name -match '(?i)vpn') -or ($desc -match '(?i)vpn') -or ($home -match '(?i)vpn')
                    if($hasProxy -and $isVPN){
                        $result += [pscustomobject]@{
                            id = [System.IO.Path]::GetFileNameWithoutExtension($_.Name)
                            name = $name
                            version = $j.version
                            homepage_url = $home
                            description = $desc
                            permissions = $perms
                            browser = 'Firefox'
                            has_proxy_permission = $hasProxy
                            is_vpn = $true
                            manifest_path = $_.FullName
                        }
                    }
                } else { $zip.Dispose() }
            } catch {}
        }
    }

    $result | ConvertTo-Json -Compress
    "#;

    // Run the PowerShell path fully hidden
    if let Ok(out) = run_powershell_hidden(ps) {
        if out.status.success() {
            let mut s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            // Trim a possible BOM
            if s.starts_with('\u{feff}') {
                s = s.trim_start_matches('\u{feff}').to_string();
            }
            if s.starts_with('[') {
                if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(&s) {
                    if !parsed.is_empty() {
                        return Ok(parsed);
                    }
                }
            } else if s.starts_with('{') {
                // In case a single object is returned
                if let Ok(one) = serde_json::from_str::<serde_json::Value>(&s) {
                    return Ok(vec![one]);
                }
            }
        }
    }

    // Fallback: Rust filesystem scan (kept as-is)
    // ...existing code...
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or("LOCALAPPDATA not found")?;
    let appdata_base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or("APPDATA not found")?;
    let mut vpn_extensions = Vec::new();
    let browser_paths = vec![
        ("Brave", base.clone(), "BraveSoftware\\Brave-Browser\\User Data\\Default\\extensions"),
        ("Chrome", base.clone(), "Google\\Chrome\\User Data\\Default\\extensions"),
        ("Edge", base.clone(), "Microsoft\\Edge\\User Data\\Default\\extensions"),
        ("Firefox", appdata_base.clone(), "Mozilla\\Firefox\\Profiles"),
    ];
    for (browser_name, base_path, rel_path) in browser_paths {
        let mut full_path = base_path.clone();
        full_path.push(rel_path);
        if !full_path.exists() { continue; }
        if let Ok(entries) = fs::read_dir(&full_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() { continue; }
                if let Ok(version_entries) = fs::read_dir(&path) {
                    for version_entry in version_entries.flatten() {
                        let version_path = version_entry.path();
                        let manifest_path = version_path.join("manifest.json");
                        if manifest_path.exists() {
                            if let Ok(manifest_text) = fs::read_to_string(&manifest_path) {
                                if let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&manifest_text) {
                                    if let Some(vpn_ext) = extract_vpn_extension(&manifest, &manifest_text, &manifest_path, browser_name) {
                                        vpn_extensions.push(vpn_ext);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(vpn_extensions)
}

fn perform_sync_recovery(app_handle: &tauri::AppHandle) -> Result<(), String> {
    let app_handle = app_handle.clone();
    std::thread::spawn(move || {
        println!("perform_sync_recovery: starting sync/recovery check");
        let res: Result<(), String> = (|| {
            if let Err(_) = check_task_schedule_exists() {
                let _ = create_eagle_task_schedule_simple();
            }

            let block_path = get_app_file_path(&app_handle, "blockData.json")?;
            if !block_path.exists() || is_file_corrupted(&block_path) {
                restore_default_block_data(&app_handle)?;
            }

            let prefs_path = get_app_file_path(&app_handle, "savedPreferences.json")?;
            if !prefs_path.exists() || is_file_corrupted(&prefs_path) {
                restore_default_preferences(&app_handle)?;
            }

            if let Ok(mut guard) = BLOCK_DATA_CACHE.lock() {
                if guard.is_none() && block_path.exists() {
                    if let Ok(map) = read_json_map(&block_path) {
                        *guard = Some(map);
                    }
                }
            }

            if let Ok(mut guard) = SAVED_PREFERENCES_CACHE.lock() {
                if guard.is_none() && prefs_path.exists() {
                    if let Ok(map) = read_json_map(&prefs_path) {
                        *guard = Some(map);
                    }
                }
            }

            if let Ok(guard) = BLOCK_DATA_CACHE.lock() {
                if let Some(cached_map) = guard.as_ref() {
                    match read_json_map(&block_path) {
                        Ok(stored_map) => {
                            if &stored_map != cached_map {
                                write_json_map(&block_path, cached_map)?;
                            }
                        }
                        Err(_) => {
                            write_json_map(&block_path, cached_map)?;
                        }
                    }
                }
            }

            if let Ok(guard) = SAVED_PREFERENCES_CACHE.lock() {
                if let Some(cached_prefs) = guard.as_ref() {
                    match read_json_map(&prefs_path) {
                        Ok(stored_prefs) => {
                            if &stored_prefs != cached_prefs {
                                write_json_map(&prefs_path, cached_prefs)?;
                            }
                        }
                        Err(_) => {
                            write_json_map(&prefs_path, cached_prefs)?;
                        }
                    }
                }
            }

            Ok(())
        })();

        if let Err(e) = res {
            eprintln!("perform_sync_recovery worker failed: {}", e);
        }
    });

    Ok(())
}

fn restore_default_block_data(app_handle: &tauri::AppHandle) -> Result<(), String> {
    let path = get_app_file_path(app_handle, "blockData.json")?;
    let default_data = serde_json::Map::new();
    write_json_map(&path, &default_data)?;
    
    if let Ok(mut guard) = BLOCK_DATA_CACHE.lock() {
        *guard = Some(default_data);
    }
    
    Ok(())
}

fn restore_default_preferences(app_handle: &tauri::AppHandle) -> Result<(), String> {
    let path = get_app_file_path(app_handle, "savedPreferences.json")?;
    let mut default_prefs = serde_json::Map::new();

    default_prefs.insert("delayTimeOut".to_string(), serde_json::Value::Number(serde_json::Number::from(180000u64)));
    default_prefs.insert("blockSettingsSwitch".to_string(), serde_json::Value::Bool(false));
    default_prefs.insert("enableProtectiveDNS".to_string(), serde_json::Value::Bool(false));
    default_prefs.insert("enforceSafeSearch".to_string(), serde_json::Value::Bool(false));
    
    write_json_map(&path, &default_prefs)
}

fn check_task_schedule_exists() -> Result<(), String> {
    let output = run_hidden_output("schtasks", &["/Query", "/TN", TASK_NAME])
        .map_err(|e| format!("failed to query task schedule: {}", e))?;
    
    if output.status.success() {
        Ok(())
    } else {
        Err("task schedule not found".into())
    }
}

fn is_file_corrupted(path: &std::path::PathBuf) -> bool {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            serde_json::from_str::<serde_json::Value>(&content).is_err()
        }
        Err(_) => true,
    }
}

#[tauri::command]
fn stop_settings_and_app_protection() -> Result<bool, String> {
    let _ = remove_eagle_task_schedule_simple();
    if let Some(flag) = PROTECTION_STOP.lock().map_err(|e| e.to_string())?.take() {
        flag.store(true, Ordering::SeqCst);
    }
    let mut guard = PROTECTION_HANDLE.lock().map_err(|e| e.to_string())?;
    if let Some(h) = guard.take() {
        let _ = h.join();
    }
    Ok(true)
}

fn read_preferences_for_key(app_handle: &tauri::AppHandle, key: &str) -> Result<bool, String> {
    let path = get_preferences_path(app_handle)?;
    let prefs = read_preferences_file(&path)?;
    Ok(prefs.get(key).and_then(|v| v.as_bool()).unwrap_or(false))
}

#[tauri::command]
fn close_app(app_handle: tauri::AppHandle, process_name: String) -> Result<bool, String> {
    // Do not block the command thread
    std::thread::spawn(move || {
        let base = process_name.trim().trim_end_matches(".exe").to_string();
        let target_exe = format!("{}.exe", base);

        // Try to kill the process (hidden)
        let _ = run_hidden_output("taskkill", &["/F", "/IM", &target_exe]);

        // Poll up to 60s for process to be gone (responsive 500ms step)
        let start = std::time::Instant::now();
        let mut success = false;
        while start.elapsed() < std::time::Duration::from_secs(60) {
            match get_running_process_names() {
                Ok(set) => {
                    if !set.contains(&target_exe.to_lowercase()) && !set.contains(&base.to_lowercase()) {
                        success = true;
                        break;
                    }
                }
                Err(_) => { /* ignore */ }
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }

        // Notify the UI
        let _ = app_handle.emit_all(
            "close-app-result",
            serde_json::json!({
                "processName": process_name,
                "success": success
            }),
        );
    });

    // Return immediately so the UI stays responsive
    Ok(true)
}

#[tauri::command]
fn close_invoking_window(window: tauri::Window) -> Result<(), String> {
    window.close().map_err(|e| e.to_string())
}

#[tauri::command]
fn close_overlay_window(app_handle: tauri::AppHandle){
    close_window(&app_handle, "overlay_window");
    OVERLAY_OPEN.store(false, Ordering::SeqCst);
}

#[tauri::command]
fn close_confirmation_dialog(app_handle: tauri::AppHandle){
    close_window(&app_handle, "confirmDialog");
}

fn close_window(app_handle: &tauri::AppHandle, given_label: &str){
    for (label, win) in app_handle.windows() {
        if label == given_label {
            let _ = win.close();
        }
    }
}

#[tauri::command]
fn show_delay_for_priming_deletion(setting_id: String, app_handle: tauri::AppHandle) {
    let _ = tauri::Manager::emit_all(
        &app_handle,
        "show_delay_for_prime_deletion",
        serde_json::json!({ "settingId": setting_id }),
    );
}

fn build_menu() -> Menu {
    let home = CustomMenuItem::new("menu-home".to_string(), "Home");
    let block_apps = CustomMenuItem::new("menu-block-apps".to_string(), "Block Apps");
    let block_websites = CustomMenuItem::new("menu-block-websites".to_string(), "Block Websites");
    let settings = CustomMenuItem::new("menu-settings".to_string(), "Settings");

    Menu::new()
        .add_item(home)
        .add_item(block_apps)
        .add_item(block_websites)
        .add_item(settings)
}

fn extract_exe_from_icon(icon: &str) -> String {
    let trimmed = icon.trim().trim_matches('"');
    let first_part = trimmed.split(',').next().unwrap_or(trimmed).trim();

    if let Some(fname) = std::path::Path::new(first_part)
        .file_name()
        .and_then(|os| os.to_str())
    {
        return fname.to_string();
    }

    first_part.split_whitespace().last().unwrap_or("").to_string()
}

fn parse_exe_from_command(cmd: &str) -> String {
    let s = cmd.trim().trim_matches('"');
    let first = s.split_whitespace().next().unwrap_or(s);
    let first = first.split(',').next().unwrap_or(first).trim().trim_matches('"');
    std::path::Path::new(first)
        .file_name()
        .and_then(|os| os.to_str())
        .unwrap_or("")
        .to_string()
}

fn looks_like_system_displayname(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("update") || n.contains("redistributable") || n.contains("runtime") ||
    n.contains("driver") || n.contains("package") || n.contains("patch") ||
    n.contains("microsoft visual c++") || n.contains(".net")
}

fn looks_like_windows_path(p: &str) -> bool {
    let lp = p.to_lowercase();
    lp.contains(r"\windows\") || lp.contains(r"\system32\") || lp.contains(r"\program files\windowsapps")
}

fn read_uninstall_entries(base: RegKey) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Ok(subs) = base.enum_keys().collect::<Result<Vec<_>, _>>() {
        for sub in subs {
            if let Ok(key) = base.open_subkey(&sub) {
                let display: Option<String> = key.get_value("DisplayName").ok();
                if let Some(name) = display {
                    if looks_like_system_displayname(&name) {
                        continue;
                    }

                    if let Ok(sysc) = key.get_value::<u32, _>("SystemComponent") {
                        if sysc == 1 {
                            continue;
                        }
                    }

                    if let Ok(release_type) = key.get_value::<String, _>("ReleaseType") {
                        if release_type.to_lowercase().contains("update") {
                            continue;
                        }
                    }

                    let mut process = String::new();

                    if let Ok(loc) = key.get_value::<String, _>("InstallLocation") {
                        let p = Path::new(&loc);
                        if p.is_dir() {
                            if let Ok(mut entries) = std::fs::read_dir(p).map(|r| r.filter_map(|e| e.ok()).collect::<Vec<_>>()) {
                                entries.sort_by_key(|e| e.file_name());
                                if let Some(entry) = entries.into_iter().find(|e| {
                                    e.path().extension().and_then(|x| x.to_str()).map(|ext| ext.eq_ignore_ascii_case("exe")).unwrap_or(false)
                                }) {
                                    process = entry.file_name().to_string_lossy().into_owned();
                                }
                            }
                        }
                    }

                    if process.is_empty() {
                        if let Ok(icon) = key.get_value::<String, _>("DisplayIcon") {
                            let exe = extract_exe_from_icon(&icon);
                            if !exe.is_empty() && !looks_like_windows_path(&icon) {
                                process = exe;
                            }
                        }
                    }

                    if process.is_empty() {
                        if let Ok(uninstall) = key.get_value::<String, _>("UninstallString") {
                            let exe = parse_exe_from_command(&uninstall);
                            if !exe.is_empty() && !looks_like_windows_path(&uninstall) {
                                process = exe;
                            }
                        }
                    }

                    if process.is_empty() {
                        continue;
                    }
                    if looks_like_windows_path(&process) || looks_like_system_displayname(&process) {
                        continue;
                    }

                    out.push((name, process));
                }
            }
        }
    }
    out
}

fn collect_app_paths() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let roots = [
        (RegKey::predef(HKEY_LOCAL_MACHINE), r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths"),
        (RegKey::predef(HKEY_CURRENT_USER), r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths"),
    ];

    for (root, sub) in &roots {
        if let Ok(k) = root.open_subkey(sub) {
            if let Ok(keys) = k.enum_keys().collect::<Result<Vec<_>, _>>() {
                for keyname in keys {
                    if let Ok(k2) = k.open_subkey(&keyname) {
                        if let Ok(pathval) = k2.get_value::<String, _>("").map(|s| s) {
                            let exe_name = std::path::Path::new(&pathval)
                                .file_name()
                                .and_then(|os| os.to_str())
                                .unwrap_or("")
                                .to_string();

                            let display = if keyname.to_lowercase().ends_with(".exe") {
                                keyname.trim_end_matches(".exe").to_string()
                            } else {
                                keyname.clone()
                            };
                            if !exe_name.is_empty() {
                                out.push((display, exe_name));
                            }
                        }
                    }
                }
            }
        }
    }

    out
}

#[tauri::command]
async fn get_all_installed_apps() -> Result<Vec<serde_json::Value>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let mut apps: Vec<(String, String)> = Vec::new();

        if let Ok(hklm) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall") {
            apps.extend(read_uninstall_entries(hklm));
        }

        if let Ok(hklm32) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey("SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall") {
            apps.extend(read_uninstall_entries(hklm32));
        }

        if let Ok(hkcu) = RegKey::predef(HKEY_CURRENT_USER).open_subkey("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall") {
            apps.extend(read_uninstall_entries(hkcu));
        }

        let app_paths = collect_app_paths();
        for (d, p) in app_paths { apps.push((d, p)); }

        apps.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        apps.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));

        let result: Vec<serde_json::Value> = apps.into_iter()
            .map(|(display, process)| {
                json!({
                    "displayName": display,
                    "processName": if process.is_empty() { serde_json::Value::String(String::new()) } else { serde_json::Value::String(process) }
                })
            })
            .collect();

        Ok(result)
    })
    .await
    .map_err(|e| format!("spawn_blocking join error: {}", e))?
}

fn handle_delay_changes(setting_id: String, value: Option<serde_json::Value>,app_handle: tauri::AppHandle) -> Result<(), String> {
    println!("handle_delay_changes: setting_id='{}', value={:?}", setting_id, value);

    if setting_id == DELAY_SETTINGS {
        let v = value.unwrap_or(serde_json::Value::Null);
        println!("handle_delay_changes: saving delayTimeOut = {}", v);
        save_preference(DELAY_SETTINGS.to_string(), v, app_handle.clone())?;
    }
    else if setting_id.contains("-->") {
        let parts: Vec<&str> = setting_id.splitn(2, "-->").collect();
        if parts.len() == 2 {
            let key_in_block_data = parts[0];
            let item = parts[1];

            let path = get_app_file_path(&app_handle, "blockData.json")?;
            let mut block_data = read_json_map(&path)?;

            let arr_val = block_data
                .entry(key_in_block_data.to_string())
                .or_insert(serde_json::Value::Array(Vec::new()));

            if let serde_json::Value::Array(ref mut arr) = arr_val {
                let exists = arr.iter().any(|v| v.as_str().map(|s| s == item).unwrap_or(false));
                if !exists {
                    arr.push(serde_json::Value::String(item.to_string()));
                    write_json_map(&path, &block_data)?;
                    println!("handle_delay_changes: appended '{}' to {}", item, key_in_block_data);
                } else {
                    println!("handle_delay_changes: '{}' already present in {}", item, key_in_block_data);
                }
            } 
            else {
                block_data.insert(
                    key_in_block_data.to_string(),
                    serde_json::Value::Array(vec![serde_json::Value::String(item.to_string())]),
                );
                write_json_map(&path, &block_data)?;
                println!("handle_delay_changes: created {} with '{}'", key_in_block_data, item);
            }

            let _ = tauri::Manager::emit_all(
                &app_handle,
                "block-data-updated",
                serde_json::json!({}),
            );

        } else {
            eprintln!("handle_delay_changes: malformed setting_id with delimiter: {}", setting_id);
        }
    }
    else {
        println!("handle_delay_changes: setting '{}' -> saving false", setting_id);
        save_preference(setting_id.clone(), serde_json::Value::Bool(false), app_handle.clone())?;
    }

    let _ = tauri::Manager::emit_all(
        &app_handle,
        "turn-off-setting",
        serde_json::json!({ "settingId": setting_id }),
    );

    Ok(())
}

#[tauri::command]
fn start_countdown_timer(setting_id: String, remaining_time: Option<u64>, target_timeout: Option<u64>, app_handle: tauri::AppHandle) -> Result<(), String> {
    if let Some((stop_flag, handle, _end_ts)) = ACTIVE_TIMERS.lock().map_err(|e| e.to_string())?.remove(&setting_id){
        println!("start_countdown_timer: stopping existing timer for '{}'", &setting_id);
        stop_flag.store(true, Ordering::SeqCst);
        let _ = handle.join();
    }

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .map_err(|e| e.to_string())?;

    let effective_delay = remaining_time.unwrap_or(get_delay_time_out(app_handle.clone())?);

    let path = get_app_file_path(&app_handle, "savedPreferences.json")?;
    let mut prefs = read_json_map(&path)?;
    let existing_entry = prefs
        .get("timerInfo")
        .and_then(|v| v.as_object())
        .and_then(|obj| obj.get(&setting_id))
        .and_then(|v| v.as_object());

    let mut should_persist = remaining_time.is_none();
    let mut start_ts_to_use = now_ms;

    if let Some(rem) = remaining_time {
        if let Some(entry) = existing_entry {
            if let Some(ts) = entry
                .get("startTimeStamp")
                .and_then(|v| v.as_u64())
                .or_else(|| entry.get("startTimeStamp").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok()))
            {
                start_ts_to_use = ts;
            }
            println!(
                "start_countdown_timer: resuming '{}' with {} ms remaining (preserving persisted startTimeStamp)",
                setting_id, rem
            );
        } 
        else {
            let saved_delay = get_delay_time_out(app_handle.clone()).unwrap_or(effective_delay);
            start_ts_to_use = now_ms.saturating_sub(saved_delay.saturating_sub(rem));
            should_persist = true;
            println!(
                "start_countdown_timer: resuming '{}' without existing entry; reconstructing startTimeStamp={}",
                setting_id, start_ts_to_use
            );
        }
    } else {
        start_ts_to_use = now_ms;
    }

    if should_persist {
        let mut timer_info = prefs.get("timerInfo").cloned().unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
        if let serde_json::Value::Object(ref mut obj) = timer_info {
            let mut m = serde_json::Map::new();
            m.insert(
                "startTimeStamp".to_string(),
                serde_json::Value::Number(serde_json::Number::from(start_ts_to_use)),
            );
            if let Some(tn) = target_timeout {
                m.insert("targetTimeout".to_string(), serde_json::Value::Number(serde_json::Number::from(tn)));
            } else {
                m.insert("targetTimeout".to_string(), serde_json::Value::Null);
            }
            let configured_timeout_at_change = get_delay_time_out(app_handle.clone()).unwrap_or(effective_delay);
            m.insert(
                DELAY_TIMEOUT_KEY.to_string(),
                serde_json::Value::Number(serde_json::Number::from(configured_timeout_at_change)),
            );

            obj.insert(setting_id.clone(), serde_json::Value::Object(m));
            prefs.insert("timerInfo".to_string(), serde_json::Value::Object(obj.clone()));
        }
        write_json_map(&path, &prefs)?;
        println!("start_countdown_timer: persisted timerInfo for '{}'", setting_id);
    } else {
        println!("start_countdown_timer: skipping persistence for '{}' (resume)", setting_id);
    }

    let end_ts = match remaining_time {
        Some(rem) => now_ms.saturating_add(rem),
        None => start_ts_to_use.saturating_add(effective_delay),
    };

    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_clone = stop_flag.clone();
    let app_clone = app_handle.clone();
    let other_clone = app_handle.clone();
    let sid = setting_id.clone();
    let tt_clone = target_timeout;

    let handle = std::thread::spawn(move || {
        loop {
            if stop_flag_clone.load(Ordering::SeqCst) {
                println!("timer thread: '{}' received stop signal", sid);
                break;
            }
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
            if now >= end_ts {
                println!("timer thread: '{}' expired", sid);

                let persisted_target_opt: Option<u64> = get_app_file_path(&app_clone, "savedPreferences.json")
                    .ok()
                    .and_then(|p| read_json_map(&p).ok())
                    .and_then(|prefs_read| {
                        prefs_read.get("timerInfo")
                            .and_then(|ti| ti.as_object())
                            .and_then(|map| map.get(&sid))
                            .and_then(|entry| entry.get("targetTimeout"))
                            .and_then(|v| v.as_u64())
                    });

                if let Ok(p) = get_app_file_path(&app_clone, "savedPreferences.json") {
                    if let Ok(mut prefs2) = read_json_map(&p) {
                        if let Some(serde_json::Value::Object(ref mut ti)) = prefs2.get_mut("timerInfo") {
                            ti.remove(&sid);
                        }
                        let _ = write_json_map(&p, &prefs2);
                    }
                }

                if let Err(e) = handle_delay_changes(
                    sid.clone(),
                    persisted_target_opt.map(|n| serde_json::Value::Number(serde_json::Number::from(n))),
                    app_clone.clone()
                ) {
                    eprintln!("timer thread: '{}' failed to handle changes: {}", sid, e);
                }

                let payload = match tt_clone {
                    Some(n) => serde_json::json!({ "settingId": sid, "targetTimeout": n }),
                    None => serde_json::json!({ "settingId": sid, "targetTimeout": serde_json::Value::Null }),
                };
                let _ = tauri::Manager::emit_all(&app_clone, "timer-expired", payload);

                let _ = ACTIVE_TIMERS.lock().map_err(|e| e.to_string()).and_then(|mut m| { m.remove(&sid); Ok(()) });
                break;
            }

            let remaining_ms = end_ts.saturating_sub(now);
            println!("timer thread: '{}' remaining {}s", sid, remaining_ms / 1000);
            std::thread::sleep(Duration::from_secs(1));
        }
    });

    ACTIVE_TIMERS.lock().map_err(|e| e.to_string())?.insert(setting_id, (stop_flag, handle, end_ts));
    let _ = tauri::Manager::emit_all(&other_clone, "timer-updated", serde_json::json!({}));
    Ok(())
}

#[tauri::command]
fn cancel_countdown_timer(setting_id: String, app_handle: tauri::AppHandle) -> Result<bool, String> {
    println!("ending countdown timer for {}", setting_id);
    if let Some((stop_flag, handle, _end_ts)) =
        ACTIVE_TIMERS.lock().map_err(|e| e.to_string())?.remove(&setting_id)
    {
        stop_flag.store(true, Ordering::SeqCst);
        let _ = handle.join();
    }

    let path = get_app_file_path(&app_handle, "savedPreferences.json")?;
    if let Ok(mut prefs) = read_json_map(&path) {
        if let Some(serde_json::Value::Object(ref mut ti)) = prefs.get_mut("timerInfo") {
            ti.remove(&setting_id);
        }
        let _ = write_json_map(&path, &prefs);
    }

    let app_clone = app_handle.clone();
    let _ = tauri::Manager::emit_all(&app_clone, "timer-updated", serde_json::json!({}));

    Ok(true)
}

#[tauri::command]
fn get_change_status(setting_id: String, app_handle: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let current_timeout = get_delay_time_out(app_handle.clone())?;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .map_err(|e| e.to_string())?;

    match ACTIVE_TIMERS.lock() {
        Ok(map) => {
            let _keys: Vec<String> = map.keys().cloned().collect();

            if let Some((_, _, end_ts)) = map.get(&setting_id) {
                let remaining = if *end_ts > now_ms { *end_ts - now_ms } else { 0u64 };
                let payload = json!({
                    "currentTimeout": current_timeout,
                    "isChanging": true,
                    "timeRemaining": remaining,
                    "newValue": serde_json::Value::Null,
                    "delayTimeOutAtTimeOfChange": serde_json::Value::Null
                });
                return Ok(payload);
            }

            for (k, (_stop, _h, end_ts)) in map.iter() {
                if k.eq_ignore_ascii_case(&setting_id) {
                    let remaining = if *end_ts > now_ms { *end_ts - now_ms } else { 0u64 };

                    let delay_at_change = get_app_file_path(&app_handle, "savedPreferences.json")
                        .ok()
                        .and_then(|p| read_json_map(&p).ok())
                        .and_then(|prefs_read| {
                            prefs_read.get("timerInfo")
                                .and_then(|ti| ti.as_object())
                                .and_then(|map| map.get(k))
                                .and_then(|entry| {
                                    entry.get(DELAY_TIMEOUT_KEY)
                                         .cloned()
                                         .or_else(|| entry.get("delayTimeoutAtTimeOfChange").cloned())
                                })
                        })
                        .unwrap_or(serde_json::Value::Null);

                    let payload = json!({
                        "currentTimeout": current_timeout,
                        "isChanging": true,
                        "timeRemaining": remaining,
                        "newValue": serde_json::Value::Null,
                        "delayTimeOutAtTimeOfChange": delay_at_change
                    });
                    return Ok(payload);
                }
            }
        }
        Err(e) => eprintln!("get_change_status: failed to lock ACTIVE_TIMERS: {}", e),
    }

    let path = get_app_file_path(&app_handle, "savedPreferences.json")?;
    let prefs = read_json_map(&path)?;

    if let Some(serde_json::Value::Object(timer_map)) = prefs.get("timerInfo") {
        let mut matched_key: Option<String> = None;
        if timer_map.contains_key(&setting_id) {
            matched_key = Some(setting_id.clone());
        } else {
            for k in timer_map.keys() {
                if k.eq_ignore_ascii_case(&setting_id) {
                    matched_key = Some(k.clone());
                    break;
                }
            }
        }

        if let Some(key) = matched_key {
            if let Some(entry) = timer_map.get(&key) {
                let start_ts = entry
                    .get("startTimeStamp")
                    .and_then(|v| v.as_u64())
                    .or_else(|| entry.get("startTimeStamp").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok()))
                    .unwrap_or(now_ms);

                 let delay_ms = entry
                    .get(DELAY_TIMEOUT_KEY)
                    .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok())))
                    .or_else(|| {
                        entry
                            .get("delayTimeoutAtTimeOfChange")
                            .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok())))
                    })
                    .unwrap_or(current_timeout);

                let end_ts = start_ts.saturating_add(delay_ms);
                let remaining = if end_ts > now_ms { end_ts - now_ms } else { 0u64 };

                let new_value = entry
                    .get("targetTimeout")
                    .cloned()
                    .or_else(|| entry.get("newDelayValue").cloned())
                    .unwrap_or(serde_json::Value::Null);

                let delay_at_change = entry
                    .get("delayTimeOutAtTimeOfChange")
                    .cloned()
                    .or_else(|| entry.get("delayTimeoutAtTimeOfChange").cloned())
                    .unwrap_or(serde_json::Value::Null);

                let payload = json!({
                    "currentTimeout": current_timeout,
                    "isChanging": true,
                    "timeRemaining": remaining,
                    "newValue": new_value,
                    "delayTimeOutAtTimeOfChange": delay_at_change
                });

                return Ok(payload);
            }
        }
    }

    let payload = json!({
        "currentTimeout": current_timeout,
        "isChanging": false,
        "delayTimeOutAtTimeOfChange": serde_json::Value::Null
    });

    Ok(payload)
}

#[tauri::command]
fn get_delay_change_status(app_handle: tauri::AppHandle) -> Result<serde_json::Value, String> {
    get_change_status(DELAY_SETTINGS.to_string(), app_handle)
}

fn menu_id_to_page(id: &str) -> Option<&'static str> {
    match id {
      "menu-home" => Some("index.html"),
      "menu-block-apps" => Some("blockApps.html"),
      "menu-block-websites" => Some("blockWebsites.html"),
      "menu-settings" => Some("delaySettings.html"),
      _ => None,
    }
}

#[tauri::command]
async fn add_block_website(site: String, app_handle: tauri::AppHandle) -> Result<bool, String> {
    let site = site.trim();
    if site.is_empty() {
        return Err("empty site".into());
    }

    let entry = format!("127.0.0.1 {}", site);
    let current = std::fs::read_to_string(HOSTS_PATH).unwrap_or_default();
    let mut has_added = false;

    if !current.contains(&entry) {
        let mut tmp_path = env::temp_dir();
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis().to_string())
            .unwrap_or_else(|_| "tmp".into());
        tmp_path.push(format!("eagleblocker_hosts_{}.tmp", suffix));
        let tmp_path_str = tmp_path.to_string_lossy().into_owned();

        println!("add_block_website: writing temp hosts to {}", tmp_path_str);

        let mut f = File::create(&tmp_path).map_err(|e| format!("failed to create temp hosts file: {}", e))?;
        f.write_all(current.as_bytes()).map_err(|e| format!("failed to write current hosts to temp file: {}", e))?;
        if !current.ends_with('\n') {
            f.write_all(b"\n").map_err(|e| format!("failed to write newline to temp file: {}", e))?;
        }
        f.write_all(entry.as_bytes()).map_err(|e| format!("failed to append entry to temp file: {}", e))?;
        f.write_all(b"\r\n").map_err(|e| format!("failed to append newline to temp file: {}", e))?;

        let cmd = format!("move /Y \"{}\" \"{}\"", tmp_path_str.replace('"', ""), HOSTS_PATH);
        println!("add_block_website: requesting elevation to run: {}", cmd);

        let _ = run_elevated_command(cmd).await.map_err(|e| {
            format!("elevated hosts update failed: {}", e)
        })?;

        println!("add_block_website: hosts updated successfully");
        has_added = true;
    } else {
        println!("add_block_website: hosts already contains entry for '{}'", site);
    }

    if !has_added {
        return Ok(false);
    }

    let path = get_app_file_path(&app_handle, "blockData.json")?;
    let mut block_data = read_json_map(&path)?;
    let mut should_write = false;

    let key = "blockedWebsites";
    let arr_val = block_data
        .entry(key.to_string())
        .or_insert(Value::Array(Vec::new()));

    if let Value::Array(ref mut arr) = arr_val {
        let exists = arr.iter().any(|v| v.as_str().map(|s| s == site).unwrap_or(false));
        if !exists {
            arr.push(Value::String(site.to_string()));
            should_write = true;
            println!("add_block_website: appended '{}' to {}", site, key);
        } else {
            println!("add_block_website: '{}' already present in {}", site, key);
        }
    } else {
        block_data.insert(
            key.to_string(),
            Value::Array(vec![Value::String(site.to_string())]),
        );
        should_write = true;
        println!("add_block_website: created {} with '{}'", key, site);
    }

    if should_write {
        write_json_map(&path, &block_data)?;
        println!("add_block_website: appended '{}' to {}", site, key);
        store_block_data_cached(&app_handle, &block_data)?;
    }

    let _ = tauri::Manager::emit_all(
        &app_handle,
        "block-data-updated",
        serde_json::json!({ "key": key, "item": site }),
    );

    Ok(true)
}

#[tauri::command]
async fn remove_block_website(site: String, app_handle: tauri::AppHandle) -> Result<bool, String> {
    let site = site.trim();
    if site.is_empty() {
        return Err("empty site".into());
    }

    println!("remove_block_website: removing {}", site);

    let current = std::fs::read_to_string(HOSTS_PATH).map_err(|e| format!("failed to read hosts file: {}", e))?;

    let pattern = format!(r"(?m)^\s*127\.0\.0\.1\s+{}\b.*\r?\n?", regex::escape(site));
    let re = Regex::new(&pattern).map_err(|e| e.to_string())?;
    let new_content = re.replace_all(&current, "").to_string();

    if new_content == current {
        println!("remove_block_website: hosts had no entry for '{}', skipping hosts edit", site);
    } 
    else {
        let mut tmp_path = env::temp_dir();
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis().to_string()).unwrap_or_else(|_| "tmp".into());
        tmp_path.push(format!("eagleblocker_hosts_rm_{}.tmp", suffix));
        let tmp_path_str = tmp_path.to_string_lossy().into_owned();

        std::fs::write(&tmp_path, new_content.as_bytes()).map_err(|e| format!("failed to write temp hosts file: {}", e))?;
        let cmd = format!("move /Y \"{}\" \"{}\"", tmp_path_str.replace('"', ""), HOSTS_PATH);

        if let Err(e) = run_elevated_command(cmd).await {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(format!("elevated move failed: {}", e));
        }

        let after = std::fs::read_to_string(HOSTS_PATH).map_err(|e| format!("failed to read hosts after elevation: {}", e))?;
        if re.is_match(&after) {
            return Err("hosts file still contains entry after elevated update".into());
        }

        println!("remove_block_website: hosts updated successfully for '{}'", site);
    }

    let path = get_app_file_path(&app_handle, "blockData.json")?;
    let mut block_data = read_json_map(&path)?;
    let mut changed = false;

    if let Some(serde_json::Value::Array(ref mut arr)) = block_data.get_mut("blockedWebsites") {
        let before = arr.len();
        arr.retain(|v| v.as_str().map(|s| s != site).unwrap_or(true));
        if arr.len() != before {
            println!("remove_block_website: removed '{}' from blockedWebsites", site);
            changed = true;
        } else {
            println!("remove_block_website: '{}' not found in blockedWebsites", site);
        }
    }

    if let Some(serde_json::Value::Array(ref mut arr)) = block_data.get_mut("allowedForUnblockWebsites") {
        let before = arr.len();
        arr.retain(|v| v.as_str().map(|s| s != site).unwrap_or(true));
        if arr.len() != before {
            println!("remove_block_website: removed '{}' from allowedForUnblockWebsites", site);
            changed = true;
        }
    }

    if changed {
        write_json_map(&path, &block_data)?;
        store_block_data_cached(&app_handle, &block_data)?;
        let _ = tauri::Manager::emit_all(
            &app_handle,
            "block-data-updated",
            serde_json::json!({ "key": "blockedWebsites", "item": site }),
        );
    }

    Ok(true)
}

#[tauri::command]
fn prime_for_deletion(item_type: String, name: String, app_handle: tauri::AppHandle) -> Result<bool, String> {
    let kind = item_type.to_lowercase();
    let key = if kind == "website" { "allowedForUnblockWebsites" } else { "allowedForUnblockApps" };

    let setting_id = format!("{}-->{}", key, name);

    println!("prime_for_deletion: priming '{}' for deletion (setting_id='{}')", name, setting_id);

    start_countdown_timer(setting_id.clone(), None, None, app_handle.clone())
        .map_err(|e| format!("prime_for_deletion: failed to start timer: {}", e))?;

    Ok(true)
}

fn reactivate_timers(app_handle: &tauri::AppHandle) -> Result<(), String> {
    let path = get_app_file_path(app_handle, "savedPreferences.json")?;
    let prefs = read_json_map(&path)?;

    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);

    if let Some(Value::Object(timer_map)) = prefs.get("timerInfo") {
        for (setting_id, entry_val) in timer_map.iter() {
            if let Some(entry) = entry_val.as_object() {
                let start_ts = entry.get("startTimeStamp")
                    .and_then(|v| v.as_u64())
                    .or_else(|| entry.get("startTimeStamp").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok()))
                    .unwrap_or(0);

                let delay_ms = entry.get("delayTimeOutAtTimeOfChange")
                    .and_then(|v| v.as_u64())
                    .or_else(|| entry.get("delayTimeOutAtTimeOfChange").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok()))
                    .or_else(|| entry.get("delayTimeoutAtTimeOfChange")
                        .and_then(|v| v.as_u64())
                        .or_else(|| entry.get("delayTimeoutAtTimeOfChange").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok())))
                    .unwrap_or_else(|| get_delay_time_out(app_handle.clone()).unwrap_or(0));

                if start_ts == 0 || delay_ms == 0 {
                    println!("reactivate_timers: skipping malformed timer '{}'", setting_id);
                    continue;
                }

                let elapsed = now_ms.saturating_sub(start_ts);
                if delay_ms > elapsed {
                    let remaining = delay_ms - elapsed;
                    println!("reactivate_timers: restarting timer '{}' with {} ms remaining", setting_id, remaining);

                    let target_opt_u64: Option<u64> = entry.get("targetTimeout")
                        .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok())));

                    let _ = start_countdown_timer(
                        setting_id.clone(),
                        Some(remaining),
                        target_opt_u64,
                        app_handle.clone()
                    ).map_err(|e| eprintln!("reactivate_timers: failed to start timer '{}': {}", setting_id, e));
                } else {
                    println!("reactivate_timers: timer '{}' already expired; handling expiration", setting_id);
                    let target_val = entry.get("targetTimeout").cloned().or_else(|| entry.get("newDelayValue").cloned());
                    if let Err(e) = handle_delay_changes(setting_id.clone(), target_val, app_handle.clone()) {
                        eprintln!("reactivate_timers: handle_delay_changes failed for '{}': {}", setting_id, e);
                    } else {
                        let _ = (|| -> Result<(), String> {
                            let mut prefs2 = read_json_map(&path)?;
                            if let Some(Value::Object(ref mut ti)) = prefs2.get_mut("timerInfo") {
                                ti.remove(setting_id);
                            }
                            write_json_map(&path, &prefs2)?;
                            Ok(())
                        })().map_err(|e| eprintln!("reactivate_timers: failed to clear expired timer '{}': {}", setting_id, e));
                    }
                }
            } else {
                println!("reactivate_timers: skipping non-object timer entry '{}'", setting_id);
            }
        }
    }

    Ok(())
}

fn resolve_app_exe_path() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe failed: {}", e))?;
    Ok(exe.to_string_lossy().into_owned())
}

fn get_user_eagle_dir() -> Result<PathBuf, String> {
    let base = std::env::var_os("APPDATA").ok_or("APPDATA not found")?;
    let mut dir = PathBuf::from(base);
    dir.push("EagleBlocker");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create dir failed: {}", e))?;
    Ok(dir)
}

fn ensure_hidden_launcher_vbs() -> Result<PathBuf, String> {
    let exe = resolve_app_exe_path()?;
    let mut vbs = get_user_eagle_dir()?;
    vbs.push("launch_eagle_hidden.vbs");

    let content = format!(
        r#"Set sh = CreateObject("WScript.Shell")
sh.Run """" & "{exe}" & """", 0, False
"#,
        exe = exe.replace('"', "\"\"")
    );

    std::fs::write(&vbs, content).map_err(|e| format!("write vbs failed: {}", e))?;
    Ok(vbs)
}

fn create_eagle_task_schedule_simple() -> Result<bool, String> {
    let vbs_path = ensure_hidden_launcher_vbs()?;
    let vbs_str = vbs_path.to_string_lossy().into_owned();

    let tr_value = format!("wscript.exe //nologo \"{}\"", vbs_str.replace('"', "\\\""));

    let args = [
        "/Create",
        "/F",
        "/SC",
        "MINUTE",
        "/MO",
        "1",
        "/TN",
        TASK_NAME,
        "/TR",
        &tr_value,
    ];

    let output = run_hidden_output("schtasks", &args).map_err(|e| format!("failed to spawn schtasks: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() {
        Ok(true)
    } else {
        Err(format!("schtasks failed: code={:?}, stdout={}, stderr={}", output.status.code(), stdout, stderr))
    }
}

fn remove_eagle_task_schedule_simple() -> Result<bool, String> {
    let args = ["/Delete", "/TN", TASK_NAME, "/F"];
    println!("remove_eagle_task_schedule_simple: running: schtasks {}", args.join(" "));
    let output = run_hidden_output("schtasks", &args)
        .map_err(|e| format!("failed to spawn schtasks: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() {
        println!("remove_eagle_task_schedule_simple: succeeded: {}", stdout);
        Ok(true)
    } else {
        Err(format!("schtasks delete failed: code={:?}, stdout={}, stderr={}", output.status.code(), stdout, stderr))
    }
}

fn register_page_change_menu_handler(app: &tauri::App) {
    let app_handle = app.handle();
    let app_handle_clone = app_handle.clone();

    app.listen_global("page-changed", move |event| {
        if let Some(payload) = event.payload() {
            if let Ok(mut cp) = CURRENT_PAGE.lock() {
                *cp = payload.to_string();
                println!("register_page_change_menu_handler: current page = {}", payload);
            }
            let _ = app_handle_clone.emit_all("page-changed-ack", None::<()>);
        }
    });
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[tauri::command]
fn show_overlay(app_handle: &tauri::AppHandle, arguments: serde_json::Value) -> Result<(), String> {
    if let Some(win) = app_handle.get_window("overlay_window") {
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(());
    }

    let obj = arguments.as_object();

    let display = obj
        .and_then(|m| m.get("displayName"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let process = obj
        .and_then(|m| m.get("processName"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let code = obj
        .and_then(|m| m.get("code"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let url = format!(
        "overlayWindow.html?code={}&displayName={}&processName={}",
        percent_encode(&code),
        percent_encode(&display),
        percent_encode(&process)
    );

    if let Err(e) = tauri::WindowBuilder::new(app_handle, "overlay_window", tauri::WindowUrl::App(url.into()))
        .title("Overlay")
        .fullscreen(false)
        .decorations(false)
        .always_on_top(false)
        .focused(true)
        .visible(true)
        .build()
    {
        eprintln!("show_overlay: failed to create overlay window: {}", e);
    }

    OVERLAY_OPEN.store(true, Ordering::SeqCst);
    Ok(())
}

fn load_block_data_cached(app_handle: &tauri::AppHandle) -> Result<Map<String, Value>, String> {
    let mut guard = BLOCK_DATA_CACHE.lock().map_err(|e| e.to_string())?;
    if let Some(m) = guard.as_ref() {
        return Ok(m.clone());
    }
    let path = get_app_file_path(app_handle, "blockData.json")?;
    let map = read_json_map(&path)?;
    *guard = Some(map.clone());
    Ok(map)
}

fn store_block_data_cached(_app_handle: &tauri::AppHandle, data: &Map<String, Value>) -> Result<(), String> {
    let mut guard = BLOCK_DATA_CACHE.lock().map_err(|e| e.to_string())?;
    *guard = Some(data.clone());
    Ok(())
}

#[tauri::command]
fn get_block_data_for_block_websites(app_handle: tauri::AppHandle) -> Result<Map<String, Value>, String> {
    load_block_data_cached(&app_handle)
}

fn ensure_overlay_below_main(app_handle: &tauri::AppHandle) {
    if let (Some(main), Some(overlay)) = (
        app_handle.get_window("main"),
        app_handle.get_window("overlay_window"),
    ) {
        let _ = overlay.set_always_on_top(true);
        let _ = main.set_always_on_top(true);
        let _ = main.set_focus();
    }
}

fn purge_old_powershell_task(task_name: &str) {
    if let Ok(output) = run_hidden_output("schtasks", &["/Query", "/TN", task_name, "/V", "/FO", "LIST"]) {
        if output.status.success() {
            let txt = String::from_utf8_lossy(&output.stdout).to_lowercase();
            if txt.contains("task to run:") && txt.contains("powershell.exe") {
                let _ = run_hidden_output("schtasks", &["/Delete", "/TN", task_name, "/F"]);
                println!("purged task '{}' that launched powershell.exe", task_name);
            }
        }
    }
}

fn main() {
    const LOCK_ADDR: &str = "127.0.0.1:58859";
    let _lock = match TcpListener::bind(LOCK_ADDR) {
        Ok(l) => l,
        Err(_) => {
            println!("Another instance is running; exiting.");
            return;
        }
    };

    tauri::Builder::default()
        .setup(|app| {
            purge_old_powershell_task("EagleElevate");
            let app_handle = app.app_handle();
            if let Err(e) = reactivate_timers(&app_handle) {
                eprintln!("reactivate_timers failed during setup: {}", e);
            }
            register_page_change_menu_handler(app);
            Ok(())
        })
        .menu(build_menu())
        .on_window_event(|event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event.event() {
                let label  = event.window().label();
                if label == "main1" || label == "overlay_window1" {
                    let should_block = read_preferences_for_key(&event.window().app_handle(), "blockSettingsSwitch").unwrap_or(false);
                    if should_block {
                        api.prevent_close();
                        tauri::api::dialog::message(
                            Some(&event.window()),
                            "Action blocked",
                            "You can’t close the app while Settings and App Protection is ON. Turn it off in Settings to quit."
                        );
                    }
                }
            }

            if let WindowEvent::Focused(true) = event.event() {
                if event.window().label() == "main" {
                    let app = event.window().app_handle();
                    ensure_overlay_below_main(&app);
                }
            }
        })
        .on_menu_event(|event: WindowMenuEvent| {
            let id = event.menu_item_id();
            let _ = tauri::Manager::emit_all(&event.window().app_handle(), "menu-event", id.to_string());

            if let Some(page) = menu_id_to_page(id) {
                let app = event.window().app_handle();
                if let Some(main_win) = app.get_window("main") {
                    let safe_page = page.replace('"', "%22");
                    let js = format!("window.location.href = \"{}\";", safe_page);
                    let _ = main_win.eval(&js);
                } else {
                    eprintln!("menu: main window not found, renderer should handle navigation for '{}'", page);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            save_preference, 
            read_preference, 
            is_dns_made_safe,
            turn_on_dns,
            is_safe_search_enabled,
            enable_safe_search,
            turn_on_settings_and_app_protection,
            close_app,
            get_block_data,
            save_block_data,
            get_all_installed_apps,
            get_delay_time_out,
            start_countdown_timer,
            cancel_countdown_timer,
            get_delay_change_status,
            stop_settings_and_app_protection,
            add_block_website,
            get_change_status,
            prime_for_deletion,
            remove_block_website,
            close_overlay_window,
            show_delay_for_priming_deletion,
            close_invoking_window,
            close_confirmation_dialog,
            get_block_data_for_block_websites
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

