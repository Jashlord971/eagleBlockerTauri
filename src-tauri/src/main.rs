// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
use once_cell::sync::Lazy;
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

// Windows-only: creation flag to prevent flashing console windows for child processes
#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

// Run a program with arguments while hiding the console window on Windows.
// Returns the Output so callers can inspect stdout/stderr.
fn run_hidden_output(program: &str, args: &[&str]) -> Result<std::process::Output, String> {
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

const REQUIRED_ENTRIES : [&str; 7]= [
    "216.239.38.120 www.google.com",
    "216.239.38.120 google.com",
    "204.79.197.220 bing.com",
    "204.79.197.220 www.bing.com",
    "213.180.193.56 yandex.ru",
    "213.180.204.92 www.yandex.com",
    "127.0.0.1 yandex.com/images",
];

const TASK_NAME: &str = "Eagle Task Schedule";

const HOSTS_PATH: &str = r"C:\Windows\System32\drivers\etc\hosts";
const DELAY_SETTINGS: &str = "delayTimeOut";

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

    tauri::Manager::emit_all(&app_handle, "preferences-updated", serde_json::Value::Null).ok();

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
            // no salvage possible
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
    let interface_name = get_active_interface_name()?;
    is_safe_dns(&interface_name)
}

#[tauri::command]
fn turn_on_dns(is_strict: bool, app_handle: tauri::AppHandle) -> Result<(), String> {
    let interface_name = get_active_interface_name()?;

    configure_safe_dns(&interface_name, is_strict).map_err(|e| {
        eprintln!("turn_on_dns: configure_safe_dns failed: {}", e);
        let elow = e.to_lowercase();
        if elow.contains("elevation canceled") || elow.contains("canceled by the user") || elow.contains("operation was canceled") {
            "elevation-canceled-by-user".to_string()
        } else {
            format!("configure_safe_dns failed: {}", e)
        }
    })?;

    save_preference(
        "enableProtectiveDNS".to_string(),
        serde_json::Value::Bool(true),
        app_handle,
    )?;

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

fn run_elevated_command(cmd: &str) -> Result<(), String> {
    // Hide the PowerShell window and UAC parent console while invoking elevation
    let ps = format!(
        "Start-Process -FilePath 'cmd.exe' -ArgumentList '/C','{}' -Verb RunAs -WindowStyle Hidden -Wait; exit $LASTEXITCODE",
        cmd.replace('\'', r#"'"#)
    );

    let output = run_hidden_output("powershell", &["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &ps])
        .map_err(|e| format!("failed to spawn powershell: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let code = output.status.code().unwrap_or(-1);

    if output.status.success() {
        return Ok(());
    }

    if code == 1223 || stderr.to_lowercase().contains("canceled by the user") || stderr.to_lowercase().contains("operation was canceled") {
        println!("Action was canceled by the user");
        return Err("elevation canceled by user".into());
    }

    Err(format!(
        "elevated command failed: code={:?}, stdout={}, stderr={}",
        code, stdout, stderr
    ))
}

fn configure_safe_dns(interface_name: &str, is_strict: bool) -> Result<(), String> {
    let (primary_dns, secondary_dns) = if is_strict {
        ("185.228.168.168", "185.228.169.168")
    } else {
        ("208.67.222.123", "208.67.220.123")
    };

    let netsh_command = format!(
        "netsh interface ipv4 set dns name=\"{}\" static {} primary && netsh interface ipv4 add dns name=\"{}\" {} index=2",
        interface_name, primary_dns, interface_name, secondary_dns
    );

    println!("Command: {}", netsh_command);
    run_elevated_command(&netsh_command)
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
fn enable_safe_search(app_handle: tauri::AppHandle) -> Result<bool, String>{
    println!("add_safe_search_hosts: reading existing hosts file");
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

    run_elevated_command(&cmd).map_err(|e| format!("elevated move failed: {}", e))?;
    
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

    let mut names: Vec<_> = set.iter().cloned().collect();
    names.sort();
    //println!("All activeApps: {}", names.join(", "));

    Ok(set)
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

fn flag_app_overlay(app_handle: &tauri::AppHandle, display: &str, process: &str) {
    println!("flag_app_overlay: display='{}', process='{}'", display, process);
    let _ = tauri::Manager::emit_all(
        app_handle,
        "flag-app-with-overlay",
        serde_json::json!({ "displayName": display, "processName": process }),
    );
}

#[tauri::command]
fn turn_on_settings_and_app_protection(app_handle: tauri::AppHandle) -> Result<bool, String> {
    println!("turn_on_settings_and_app_protection: starting");
    let mut guard = PROTECTION_HANDLE.lock().map_err(|e| e.to_string())?;
    if guard.is_some() {
        println!("turn_on_settings_and_app_protection: already running");
        return Ok(true);
    }

    let stop_flag = Arc::new(AtomicBool::new(false));
    *PROTECTION_STOP.lock().map_err(|e| e.to_string())? = Some(stop_flag.clone());

    let app_clone = app_handle.clone();
    let handle = std::thread::spawn(move || {
        let interval = Duration::from_secs(12);
        loop {
            if stop_flag.load(Ordering::SeqCst) {
                break;
            }

            let enabled = read_preferences_for_key(&app_clone, "blockSettingsSwitch").unwrap_or(false);
            if enabled {
                match get_running_process_names() {
                    Ok(running) => {
                        if let Ok(block_map) = load_block_data(&app_clone) {
                            let blocked_apps = collect_blocked_apps(&block_map);
                            for (proc_name, display_name) in blocked_apps.iter() {
                                if process_matches_running(proc_name, &running) {
                                    flag_app_overlay(&app_clone, display_name, proc_name);
                                    std::thread::sleep(Duration::from_secs(5));
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => eprintln!("protection thread: get_running_process_names failed: {}", e),
                }
            }

            std::thread::sleep(interval);
        }
    });

    *guard = Some(handle);
    create_eagle_task_schedule_simple();
    println!("turn_on_settings_and_app_protection: protection thread started");
    Ok(true)
}

#[tauri::command]
fn stop_settings_and_app_protection() -> Result<bool, String> {
    remove_eagle_task_schedule_simple();
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
fn close_app(process_name: String) -> Result<bool, String> {
    let base = process_name.trim_end_matches(".exe");
    let ps = format!("$proc = Get-Process -Name '{}' -ErrorAction SilentlyContinue; if ($proc) {{ Stop-Process -Name '{}' -Force -ErrorAction SilentlyContinue }}", base, base);
    let _ = run_hidden_output("powershell", &["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &ps])
        .map_err(|e| format!("failed to spawn powershell: {}", e))?;

    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        if let Ok(set) = get_running_process_names() {
            if !set.contains(&format!("{}.exe", base)) && !set.contains(&base.to_string()) {
                return Ok(true);
            }
        }
        thread::sleep(Duration::from_secs(5));
    }
    Ok(false)
}

#[tauri::command]
fn close_overlay_window(app_handle: tauri::AppHandle){
    let _ = tauri::Manager::emit_all(
        &app_handle,
        "close_overlay_window_prompted",
        serde_json::json!({}),
    );
}

#[tauri::command]
fn close_confirm_modal(app_handle: tauri::AppHandle){
    let _ = tauri::Manager::emit_all(
        &app_handle,
        "close_confirm_modal_prompt",
        serde_json::json!({}),
    );
}

#[tauri::command]
fn close_dns_modal(app_handle: tauri::AppHandle){
    let _ = tauri::Manager::emit_all(
        &app_handle,
        "close_dns_modal_prompt",
        serde_json::json!({}),
    );
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
fn get_all_installed_apps() -> Result<Vec<serde_json::Value>, String> {
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
    if !app_paths.is_empty() {
        println!("get_all_installed_apps: adding {} entries from App Paths", app_paths.len());
        for (d, p) in app_paths {
            apps.push((d, p));
        }
    }

    apps.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    apps.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));

    let result: Vec<serde_json::Value> = apps.into_iter().map(|(display, process)| {
        json!({
            "displayName": display,
            "processName": if process.is_empty() { serde_json::Value::String(String::new()) } else { serde_json::Value::String(process) }
        })
    }).collect();

    Ok(result)
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
        println!("start_countdown_timer: stopping existing timer for '{}'", setting_id);
        stop_flag.store(true, Ordering::SeqCst);
        let _ = handle.join();
        println!("start_countdown_timer: existing timer for '{}' stopped", setting_id);
    }

    let effective_delay = match remaining_time {
        Some(v) => v,
        None => get_delay_time_out(app_handle.clone())?,
    };

    println!(
        "start_countdown_timer: starting timer '{}' with delay {} ms (target_timeout={:?})",
        setting_id, effective_delay, target_timeout
    );

    let start_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .map_err(|e| e.to_string())?;

    let path = get_app_file_path(&app_handle, "savedPreferences.json")?;
    let mut prefs = read_json_map(&path)?;
    let mut timer_info = match prefs.get("timerInfo") {
        Some(v) => v.clone(),
        None => serde_json::Value::Object(serde_json::Map::new()),
    };

    if let serde_json::Value::Object(ref mut obj) = timer_info {
        let mut m = serde_json::Map::new();
        m.insert(
            "startTimeStamp".to_string(),
            serde_json::Value::Number(serde_json::Number::from(start_ts)),
        );
        if let Some(tn) = target_timeout {
            m.insert("targetTimeout".to_string(), serde_json::Value::Number(serde_json::Number::from(tn)));
        } else {
            m.insert("targetTimeout".to_string(), serde_json::Value::Null);
        }
        
        let configured_timeout_at_change = get_delay_time_out(app_handle.clone()).unwrap_or(effective_delay);
        m.insert(
            "delayTimeOutAtTimeOfChange".to_string(),
            serde_json::Value::Number(serde_json::Number::from(configured_timeout_at_change)),
        );

        obj.insert(setting_id.clone(), serde_json::Value::Object(m));
        prefs.insert("timerInfo".to_string(), serde_json::Value::Object(obj.clone()));
    }

    write_json_map(&path, &prefs)?;
    println!("start_countdown_timer: persisted timerInfo for '{}'", setting_id);

    let end_ts = start_ts.saturating_add(effective_delay);

    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_clone = stop_flag.clone();
    let app_clone = app_handle.clone();
    let sid = setting_id.clone();
    let tt_clone = target_timeout; // Option<u64>

    let handle = std::thread::spawn(move || {
        loop {
            if stop_flag_clone.load(Ordering::SeqCst) {
                println!("timer thread: '{}' received stop signal", sid);
                break;
            }

            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);

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

                if sid == DELAY_SETTINGS {
                    if let Some(tv) = persisted_target_opt {
                        let value_to_save = serde_json::Value::Number(serde_json::Number::from(tv));
                        if let Err(e) = handle_delay_changes(sid.clone(), Some(value_to_save), app_clone.clone()) {
                            eprintln!("timer thread: '{}' failed to handle delay changes: {}", sid, e);
                        }
                    } else {
                        if let Err(e) = handle_delay_changes(sid.clone(), None, app_clone.clone()) {
                            eprintln!("timer thread: '{}' failed to handle delay changes (no target): {}", sid, e);
                        }
                    }
                } else {
                    if let Err(e) = handle_delay_changes(sid.clone(), None, app_clone.clone()) {
                        eprintln!("timer thread: '{}' failed to handle changes: {}", sid, e);
                    }
                }

                let payload = match tt_clone {
                    Some(n) => serde_json::json!({ "settingId": sid, "targetTimeout": n }),
                    None => serde_json::json!({ "settingId": sid, "targetTimeout": serde_json::Value::Null }),
                };
                let _ = tauri::Manager::emit_all(&app_clone, "timer-expired", payload);

                let _ = ACTIVE_TIMERS
                    .lock()
                    .map_err(|e| e.to_string())
                    .and_then(|mut m| {
                        m.remove(&sid);
                        Ok(())
                    });

                break;
            }

            let remaining_ms = end_ts.saturating_sub(now);
            let remaining_secs = remaining_ms / 1000;
            println!("timer thread: '{}' remaining {}s", sid, remaining_secs);

            std::thread::sleep(Duration::from_secs(1));
        }
    });

    ACTIVE_TIMERS
        .lock()
        .map_err(|e| e.to_string())?
        .insert(setting_id, (stop_flag, handle, end_ts));

    Ok(())
}

#[tauri::command]
fn cancel_countdown_timer(setting_id: String, app_handle: tauri::AppHandle) -> Result<bool, String> {
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
            // prefer exact match first, then case-insensitive fallback
            let mut found_key: Option<String> = None;
            let mut found_end_ts: Option<u64> = None;

            if let Some((_, _, end_ts)) = map.get(&setting_id) {
                found_key = Some(setting_id.clone());
                found_end_ts = Some(*end_ts);
            } else {
                for (k, (_f, _h, end_ts)) in map.iter() {
                    if k.eq_ignore_ascii_case(&setting_id) {
                        found_key = Some(k.clone());
                        found_end_ts = Some(*end_ts);
                        break;
                    }
                }
            }

            if let (Some(key), Some(end_ts)) = (found_key, found_end_ts) {
                let remaining = if end_ts > now_ms { end_ts - now_ms } else { 0u64 };

                let delay_at_change = get_app_file_path(&app_handle, "savedPreferences.json")
                    .ok()
                    .and_then(|p| read_json_map(&p).ok())
                    .and_then(|prefs_read| {
                        prefs_read.get("timerInfo")
                            .and_then(|ti| ti.as_object())
                            .and_then(|map| map.get(&key))
                            .and_then(|entry| {
                                entry.get("delayTimeOutAtTimeOfChange")
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
                println!("get_change_status('{}'): in-memory timer found for key='{}', returning = {}", setting_id, key, payload);
                return Ok(payload);
            }
        }
        Err(e) => eprintln!("get_change_status: failed to lock ACTIVE_TIMERS: {}", e),
    }

    let path = get_app_file_path(&app_handle, "savedPreferences.json")?;
    let prefs = read_json_map(&path)?;

    if let Ok(s) = serde_json::to_string_pretty(&serde_json::Value::Object(prefs.clone())) {
        println!("get_change_status('{}'): prefs =\n{}", setting_id, s);
    }

    if let Some(serde_json::Value::Object(timer_map)) = prefs.get("timerInfo") {
        let mut matched_key: Option<String> = None;
        if timer_map.contains_key(&setting_id) {
            matched_key = Some(setting_id.clone());
        } 
        else {
            for k in timer_map.keys() {
                if k.eq_ignore_ascii_case(&setting_id) {
                    matched_key = Some(k.clone());
                    break;
                }
            }
        }
        if matched_key.is_none() {
            for (k, v) in timer_map.iter() {
                if let Some(obj) = v.as_object() {
                    if obj.contains_key("startTimeStamp") {
                        matched_key = Some(k.clone());
                        break;
                    }
                }
            }
        }

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
                    println!("get_change_status('{}'): matched timerInfo key = '{}', entry = {}", setting_id, key, entry);

                    let delay_ms = current_timeout;

                    let start_ts = entry
                        .get("startTimeStamp")
                        .and_then(|v| v.as_u64())
                        .or_else(|| entry.get("startTimeStamp").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok()))
                        .unwrap_or(now_ms);

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

                    println!("get_change_status('{}'): returning = {}", setting_id, payload);
                    return Ok(payload);
                }
            } else {
                println!("get_change_status('{}'): no matching timerInfo key found (tried '{}')", setting_id, setting_id);
            }
        } else {
            println!("get_change_status('{}'): no timerInfo object in prefs", setting_id);
        }
    } else {
        println!("get_change_status('{}'): no timerInfo object in prefs", setting_id);
    }

    let payload = json!({
        "currentTimeout": current_timeout,
        "isChanging": false,
        "delayTimeOutAtTimeOfChange": serde_json::Value::Null
    });
    println!("get_change_status('{}'): returning = {}", setting_id, payload);
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
fn add_block_website(site: String, app_handle: tauri::AppHandle) -> Result<bool, String> {
    let site = site.trim();
    if site.is_empty() {
        return Err("empty site".into());
    }

    let entry = format!("127.0.0.1 {}", site);
    let current = std::fs::read_to_string(HOSTS_PATH).unwrap_or_default();

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

        run_elevated_command(&cmd).map_err(|e| format!("elevated hosts update failed: {}", e))?;
        println!("add_block_website: hosts updated successfully");
    } else {
        println!("add_block_website: hosts already contains entry for '{}'", site);
    }

    let path = get_app_file_path(&app_handle, "blockData.json")?;
    let mut block_data = read_json_map(&path)?;

    let key = "blockedWebsites";
    let arr_val = block_data
        .entry(key.to_string())
        .or_insert(Value::Array(Vec::new()));

    if let Value::Array(ref mut arr) = arr_val {
        let exists = arr.iter().any(|v| v.as_str().map(|s| s == site).unwrap_or(false));
        if !exists {
            arr.push(Value::String(site.to_string()));
            write_json_map(&path, &block_data)?;
            println!("add_block_website: appended '{}' to {}", site, key);
        } else {
            println!("add_block_website: '{}' already present in {}", site, key);
        }
    } else {
        block_data.insert(
            key.to_string(),
            Value::Array(vec![Value::String(site.to_string())]),
        );
        write_json_map(&path, &block_data)?;
        println!("add_block_website: created {} with '{}'", key, site);
    }

    let _ = tauri::Manager::emit_all(
        &app_handle,
        "block-data-updated",
        serde_json::json!({ "key": key, "item": site }),
    );

    Ok(true)
}

#[tauri::command]
fn remove_block_website(site: String, app_handle: tauri::AppHandle) -> Result<bool, String> {
    let site = site.trim();
    if site.is_empty() {
        return Err("empty site".into());
    }

    println!("remove_block_website: removing {}", site);

    let current = std::fs::read_to_string(HOSTS_PATH)
        .map_err(|e| format!("failed to read hosts file: {}", e))?;

    let pattern = format!(r"(?m)^\s*127\.0\.0\.1\s+{}\b.*\r?\n?", regex::escape(site));
    let re = Regex::new(&pattern).map_err(|e| e.to_string())?;
    let new_content = re.replace_all(&current, "").to_string();

    if new_content == current {
        println!("remove_block_website: hosts had no entry for '{}', skipping hosts edit", site);
    } else {
        let mut tmp_path = env::temp_dir();
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis().to_string()).unwrap_or_else(|_| "tmp".into());
        tmp_path.push(format!("eagleblocker_hosts_rm_{}.tmp", suffix));
        let tmp_path_str = tmp_path.to_string_lossy().into_owned();

        std::fs::write(&tmp_path, new_content.as_bytes()).map_err(|e| format!("failed to write temp hosts file: {}", e))?;
        let cmd = format!("move /Y \"{}\" \"{}\"", tmp_path_str.replace('"', ""), HOSTS_PATH);

        if let Err(e) = run_elevated_command(&cmd) {
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

                let delay_ms = entry.get("delayTimeout")
                    .and_then(|v| v.as_u64())
                    .or_else(|| entry.get("delayTimeout").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok()))
                    .unwrap_or_else(|| get_delay_time_out(app_handle.clone()).unwrap_or(0u64));

                if start_ts == 0 || delay_ms == 0 {
                    println!("reactivate_timers: skipping malformed timer '{}'", setting_id);
                    continue;
                }

                let elapsed = now_ms.saturating_sub(start_ts);
                if delay_ms > elapsed {
                    let remaining = delay_ms - elapsed;
                    println!("reactivate_timers: restarting timer '{}' with {} ms remaining", setting_id, remaining);

                    let target_opt_u64: Option<u64> = entry.get("targetTimeout")
                        .and_then(|v| {
                            v.as_u64().or_else(|| {
                                v.as_str().and_then(|s| s.parse::<u64>().ok())
                            })
                        });

                    let _ = start_countdown_timer(
                        setting_id.clone(),
                        Some(remaining),
                        target_opt_u64,
                        app_handle.clone()
                    ).map_err(|e| eprintln!("reactivate_timers: failed to start timer '{}': {}", setting_id, e));
                } else {
                    println!("reactivate_timers: timer '{}' already expired; handling expiration", setting_id);
                    let target_val = entry.get("targetTimeout").cloned();
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

fn create_eagle_task_schedule_simple() -> Result<bool, String> {
    let default_app_path: Option<&str> = Some(r"C:\Program Files\Eagle Blocker\Eagle Blocker.exe");

    let exe_path = if let Some(p) = default_app_path {
        p.to_string()
    } else {
        std::env::current_exe()
            .map_err(|e| format!("failed to determine current exe path: {}", e))?
            .to_string_lossy()
            .into_owned()
    };

    let tr_value = format!("\"{}\"", exe_path.replace('"', "\\\""));
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

    println!("create_eagle_task_schedule_simple: running: schtasks {}", args.join(" "));
    let output = run_hidden_output("schtasks", &args)
        .map_err(|e| format!("failed to spawn schtasks: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() {
        println!("create_eagle_task_schedule_simple: succeeded: {}", stdout);
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
                if event.window().label() == "main" {
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
            close_confirm_modal,
            close_dns_modal,
            show_delay_for_priming_deletion
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

