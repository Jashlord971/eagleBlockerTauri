use std::collections::HashSet;
use std::net::{TcpStream, SocketAddr};
use std::time::Duration;
use std::path::Path;
use winreg::{RegKey, enums::*};

pub struct BrowserDetector {
    known_browsers: HashSet<String>,
    tor_browsers: HashSet<String>,
    registered_browsers: Vec<String>,
}

impl BrowserDetector {
    pub fn new() -> Self {
        let known_browsers = [
            "chrome.exe", 
            "firefox.exe", 
            "msedge.exe", 
            "iexplore.exe",
            "opera.exe", 
            "brave.exe", 
            "vivaldi.exe", 
            "safari.exe",
            "tor.exe", 
            "waterfox.exe", 
            "palemoon.exe", 
            "seamonkey.exe",
            "yandex.exe", 
            "maxthon.exe", 
            "ucbrowser.exe",
            "360chrome.exe",
            "duckduckgo.exe", 
            "chromium.exe", 
            "edge.exe"
        ].iter().map(|s| s.to_lowercase()).collect();

        let tor_browsers = [
            "tor.exe", 
            "firefox.exe", 
            "torbrowser.exe", 
            "tor browser.exe"
        ]
                .iter()
                .map(|s| s.to_lowercase())
                .collect();

        Self {
            known_browsers,
            tor_browsers,
            registered_browsers: Self::get_registered_browsers(),
        }
    }

    fn is_known_browser(&self, process_name: &str) -> bool {
        let lowercase = process_name.to_lowercase();
        self.known_browsers
            .iter()
            .any(|browser| {
                lowercase.contains(&browser.replace(".exe", "")) || lowercase == *browser
            })
    }

    fn is_registered_browser(&self, process_name: &str) -> bool {
        let lowercase = process_name.to_lowercase();
        self.registered_browsers
            .iter()
            .any(|browser| lowercase.contains(browser))
    }

    fn is_tor_proxy_running(&self) -> bool {
        let tor_addresses = [
            "127.0.0.1:9050",
            "127.0.0.1:9150", 
            "127.0.0.1:8118",
        ];
        
        for addr_str in &tor_addresses {
            if let Ok(addr) = addr_str.parse::<SocketAddr>() {
                match TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
                    Ok(_) => {
                        println!("✓ SUCCESS: Detected Tor proxy on {}", addr_str);
                        return true;
                    }
                    Err(_e) => {}
                }
            }
        }

        false
    }

    fn find_all_listening_ports(&self) -> Vec<(String, String)> {
        let mut tor_related_ports = Vec::new();
        
        self.scan_ports_with_powershell(&mut tor_related_ports);
        
        tor_related_ports
    }

    fn is_known_false_positive(&self, line: &str) -> bool {
        let line_lower = line.to_lowercase();
        let false_positives = [
            "webstorm", 
            "intellij", 
            "pycharm", 
            "idea",
            "vscode", 
            "code", 
            "sublime",
            "node", 
            "npm", 
            "yarn",
            "webpack", 
            "babel", 
            "eslint",
            ":63342",
            ":3000", 
            ":8080",
            ":8000"
        ];
        
        false_positives.iter().any(|fp| line_lower.contains(fp))
    }

    fn run_powershell_hidden_args(args: &[&str]) -> Result<std::process::Output, String> {
        for exe in ["pwsh.exe", "powershell.exe"] {
            match crate::run_hidden_output(&exe, args) {
                Ok(out) => return Ok(out),
                Err(e) => {
                    let el = e.to_lowercase();
                    if el.contains("not found") || el.contains("cannot find") {
                        continue;
                    }
                    return Err(format!("failed to run {}: {}", exe, e));
                }
            }
        }
        Err("no PowerShell found (pwsh.exe or powershell.exe)".into())
    }

    fn scan_ports_with_powershell(&self, tor_ports: &mut Vec<(String, String)>) {
        let ps_cmd = r#"
            Get-NetTCPConnection | Where-Object {$_.State -eq "Listen"} |
            ForEach-Object {
                $proc = Get-Process -Id $_.OwningProcess -ErrorAction SilentlyContinue
                "$($_.LocalAddress):$($_.LocalPort) - $($proc.ProcessName)"
            }
        "#;

        let args = [
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy", "Bypass",
            "-WindowStyle", "Hidden",
            "-Command", ps_cmd,
        ];

        match Self::run_powershell_hidden_args(&args) {
            Ok(output) => {
                let output_str = String::from_utf8_lossy(&output.stdout);
                for line in output_str.lines() {
                    if !line.trim().is_empty()
                        && !self.is_known_false_positive(line)
                        && self.is_tor_related_line(line)
                    {
                        tor_ports.push((line.to_string(), "PowerShell Tor Detection".to_string()));
                    }
                }
            }
            Err(e) => {
                println!("Failed to run PowerShell hidden: {}", e);
            }
        }
    }
    
    fn is_tor_related_line(&self, line: &str) -> bool {
        let line_lower = line.to_lowercase();
        
        let tor_process_names = [
            "tor", 
            "firefox", 
            "torbrowser", 
            "obfs4proxy", 
            "snowflake", 
            "meek", 
            "privoxy"
        ];
        
        let tor_ports = [
            ":9050", 
            ":9051", 
            ":9150", 
            ":9151", 
            ":8118", 
            ":9001", 
            ":9030"
        ];
        
        tor_process_names.iter().any(|name| line_lower.contains(name)) ||
        tor_ports.iter().any(|port| line_lower.contains(port))
    }
    
    pub fn is_tor_proxy_running_enhanced(&self) -> bool {
        if self.is_tor_proxy_running() {
            return true;
        }
        
        let tor_ports = self.find_all_listening_ports();
        
        if !tor_ports.is_empty() {
            for (port_info, _) in &tor_ports {
                if let Some(port) = self.extract_port_from_line(port_info) {
                    let addr_str = format!("127.0.0.1:{}", port);
                    if let Ok(addr) = addr_str.parse::<SocketAddr>() {
                        if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
                            println!("✓ SUCCESS: Connected to Tor-related port {}", addr_str);
                            return true;
                        }
                    }
                }
            }
        }
        
        false
    }
    
    fn extract_port_from_line(&self, line: &str) -> Option<u16> {
        if let Some(colon_pos) = line.rfind(':') {
            let after_colon = &line[colon_pos + 1..];
            if let Some(space_pos) = after_colon.find(' ') {
                after_colon[..space_pos].parse().ok()
            } else {
                after_colon.parse().ok()
            }
        } else {
            None
        }
    }
    
    fn is_tor_proxy_configured(&self) -> bool {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(internet_settings) = hkcu.open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings") {
            if let Ok(proxy_enable) = internet_settings.get_value::<u32, _>("ProxyEnable") {
                if proxy_enable == 1 {
                    if let Ok(proxy_server) = internet_settings.get_value::<String, _>("ProxyServer") {
                        return proxy_server.contains("127.0.0.1:9050") || 
                            proxy_server.contains("127.0.0.1:9150") ||
                            proxy_server.contains("localhost:9050");
                    }
                }
            }
        }
        false
    }

    pub fn is_browser_application(&self, process_name: &str) -> bool {
        if self.is_known_browser(process_name) {
            return true;
        }
        
        if self.is_registered_browser(process_name) {
            return true;
        }
        
        let lowercase = process_name.to_lowercase();
        let browser_keywords = ["browser", "web", "chrome", "firefox", "edge", "safari", "internet"];
        browser_keywords.iter().any(|keyword| lowercase.contains(keyword))
    }

    fn get_registered_browsers() -> Vec<String> {
        let mut browsers = Vec::new();
        
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        if let Ok(browsers_key) = hklm.open_subkey("SOFTWARE\\Clients\\StartMenuInternet") {
            for browser_name in browsers_key.enum_keys().filter_map(|x| x.ok()) {
                if let Ok(browser_key) = browsers_key.open_subkey(&browser_name) {
                    if let Ok(shell_key) = browser_key.open_subkey("shell\\open\\command") {
                        if let Ok(exe_path) = shell_key.get_value::<String, _>("") {
                            if let Some(exe_name) = Path::new(&exe_path)
                                .file_name()
                                .and_then(|n| n.to_str()) {
                                browsers.push(exe_name.to_lowercase());
                            }
                        }
                    }
                }
            }
        }
        
        browsers
    }
}

impl Default for BrowserDetector {
    fn default() -> Self {
        Self::new()
    }
}