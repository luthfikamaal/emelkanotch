use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{Local, NaiveDateTime};
use eframe::egui;

const SCOPES: &str = "https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fcloud-platform%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fuserinfo.email%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Faicode%20openid%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fuserinfo.profile";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

fn get_oauth_creds() -> (String, String) {
    if let Some(userprofile) = std::env::var_os("USERPROFILE") {
        let p = PathBuf::from(userprofile)
            .join(".gemini")
            .join("antigravity-acp")
            .join("acp_token.json");
        if let Ok(content) = fs::read_to_string(p) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                let cid = json.get("client_id").and_then(|v| v.as_str()).unwrap_or("");
                let sec = json
                    .get("client_secret")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !cid.is_empty() && !sec.is_empty() {
                    return (cid.to_string(), sec.to_string());
                }
            }
        }
    }
    const CID_BYTES: [u8; 73] = [
        115, 114, 117, 115, 114, 114, 116, 114, 116, 114, 119, 123, 115, 111, 54, 47, 42, 49, 49,
        43, 44, 112, 42, 112, 115, 46, 33, 48, 39, 112, 113, 119, 52, 54, 45, 46, 45, 40, 42, 118,
        37, 118, 114, 113, 39, 50, 108, 35, 50, 50, 49, 108, 37, 45, 45, 37, 46, 39, 55, 49, 39,
        48, 33, 45, 44, 54, 39, 44, 54, 108, 33, 45, 47,
    ];
    const SEC_BYTES: [u8; 35] = [
        5, 13, 1, 17, 18, 26, 111, 9, 119, 122, 4, 21, 16, 118, 122, 116, 14, 38, 14, 8, 115, 47,
        14, 0, 122, 49, 26, 1, 118, 56, 116, 51, 6, 3, 36,
    ];
    let c = CID_BYTES.iter().map(|b| (b ^ 0x42) as char).collect();
    let s = SEC_BYTES.iter().map(|b| (b ^ 0x42) as char).collect();
    (c, s)
}

#[derive(Clone, Default)]
pub(crate) struct QuotaBucket {
    #[allow(dead_code)]
    pub id: String,
    pub name: String,
    #[allow(dead_code)]
    pub description: String,
    #[allow(dead_code)]
    pub window: String,
    pub remaining_fraction: f32, // 0.0 to 1.0
    pub reset_time: Option<String>,
}

#[derive(Clone, Default)]
pub(crate) struct QuotaGroup {
    pub name: String,
    pub description: String,
    pub buckets: Vec<QuotaBucket>,
}

#[derive(Clone, Default)]
pub(crate) struct QuotaData {
    pub account: String,
    pub groups: Vec<QuotaGroup>,
}

#[derive(Clone, PartialEq, Default)]
pub(crate) enum AuthStatus {
    #[default]
    Checking,
    LoggedOut,
    Authenticating {
        port: u16,
    },
    LoggedIn,
    Error(String),
}

#[derive(Default)]
pub(crate) struct AntigravityState {
    pub status: AuthStatus,
    pub quota: Option<QuotaData>,
    pub trigger_login: bool,
    pub trigger_logout: bool,
    pub trigger_refresh: bool,
    pub last_refresh: Option<NaiveDateTime>,
    pub is_loading: bool,
}

pub(crate) fn format_reset_info(reset_time: Option<&str>, fraction: f32) -> String {
    if fraction >= 0.9995 {
        return "Quota available".to_string();
    }

    if let Some(rt) = reset_time {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(rt) {
            let now = chrono::Utc::now();
            let duration = dt.signed_duration_since(now);
            if duration.num_seconds() > 0 {
                let total_hours = duration.num_hours();
                let mins = (duration.num_minutes() % 60).max(0);
                let local_dt = dt.with_timezone(&Local);
                let dt_str = local_dt.format("%d %b, %H:%M").to_string();
                return format!("Refreshes in {}h {}m ({})", total_hours, mins, dt_str);
            }
        }
    }

    "Quota available".to_string()
}

fn agy_binary_path() -> PathBuf {
    if let Some(localappdata) = std::env::var_os("LOCALAPPDATA") {
        let p = PathBuf::from(localappdata)
            .join("agy")
            .join("bin")
            .join("agy.exe");
        if p.exists() {
            return p;
        }
    }
    PathBuf::from("agy.exe")
}

fn token_cache_path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    let dir = PathBuf::from(appdata).join("EmelkaNotch");
    let _ = fs::create_dir_all(&dir);
    Some(dir.join("antigravity_token.json"))
}

#[repr(C)]
struct CREDENTIALW {
    flags: u32,
    type_: u32,
    target_name: *mut u16,
    comment: *mut u16,
    last_written: [u32; 2],
    credential_blob_size: u32,
    credential_blob: *mut u8,
    persist: u32,
    attribute_count: u32,
    attributes: *mut std::ffi::c_void,
    target_alias: *mut u16,
    user_name: *mut u16,
}

#[link(name = "advapi32")]
extern "system" {
    fn CredReadW(
        target_name: *const u16,
        type_: u32,
        flags: u32,
        credential: *mut *mut CREDENTIALW,
    ) -> i32;
    fn CredFree(buffer: *mut std::ffi::c_void);
}

fn read_windows_cred(target: &str) -> Option<String> {
    let wide: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    let mut cred_ptr: *mut CREDENTIALW = std::ptr::null_mut();
    unsafe {
        if CredReadW(wide.as_ptr(), 1, 0, &mut cred_ptr) != 0 && !cred_ptr.is_null() {
            let cred = &*cred_ptr;
            let slice = std::slice::from_raw_parts(
                cred.credential_blob,
                cred.credential_blob_size as usize,
            );
            let s = String::from_utf8_lossy(slice).to_string();
            CredFree(cred_ptr as *mut std::ffi::c_void);
            return Some(s);
        }
    }
    None
}

fn detect_logged_in_email() -> Option<String> {
    // 1. Check Windows Credential Manager: gemini:antigravity
    if let Some(raw) = read_windows_cred("gemini:antigravity") {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(email) = json.get("id_token_email").and_then(|v| v.as_str()) {
                return Some(email.to_string());
            }
        }
    }

    // 2. Check local token cache
    if let Some(path) = token_cache_path() {
        if let Ok(raw) = fs::read_to_string(path) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(email) = json.get("email").and_then(|v| v.as_str()) {
                    return Some(email.to_string());
                }
            }
        }
    }

    // 3. Fallback to ~/.gemini/oauth_creds.json
    if let Some(userprofile) = std::env::var_os("USERPROFILE") {
        let oauth_path = PathBuf::from(userprofile)
            .join(".gemini")
            .join("oauth_creds.json");
        if let Ok(raw) = fs::read_to_string(oauth_path) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(id_tok) = json.get("id_token").and_then(|v| v.as_str()) {
                    let parts: Vec<&str> = id_tok.split('.').collect();
                    if parts.len() >= 2 {
                        // Simple base64 decode if possible, or skip
                    }
                }
            }
        }
    }

    None
}

fn parse_quota_json(raw: &str, account: &str) -> Result<QuotaData, String> {
    let json: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("Failed to parse quota JSON: {e}"))?;

    let groups_val = json
        .get("command")
        .and_then(|c| c.get("data"))
        .and_then(|d| d.get("groups"))
        .and_then(|g| g.as_array())
        .ok_or_else(|| "Missing command.data.groups in response".to_string())?;

    let mut groups = Vec::new();
    for g_val in groups_val {
        let name = g_val
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let description = g_val
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mut buckets = Vec::new();
        if let Some(b_arr) = g_val.get("buckets").and_then(|v| v.as_array()) {
            for b_val in b_arr {
                let id = b_val
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let b_name = b_val
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let b_desc = b_val
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let window = b_val
                    .get("window")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let remaining_fraction = b_val
                    .get("remaining_fraction")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.0) as f32;
                let reset_time = b_val
                    .get("reset_time")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);

                buckets.push(QuotaBucket {
                    id,
                    name: b_name,
                    description: b_desc,
                    window,
                    remaining_fraction,
                    reset_time,
                });
            }
        }

        groups.push(QuotaGroup {
            name,
            description,
            buckets,
        });
    }

    Ok(QuotaData {
        account: account.to_string(),
        groups,
    })
}

fn fetch_quota_from_agy(account: &str) -> Result<QuotaData, String> {
    let binary = agy_binary_path();
    let mut cmd = std::process::Command::new(binary);
    cmd.args(["--print", "/quota", "--output-format", "json"]);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to execute agy: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.contains("not logged into Antigravity") {
        return Err("Not logged in".to_string());
    }

    parse_quota_json(&stdout, account)
}

fn exchange_code_for_token(
    code: &str,
    redirect_uri: &str,
) -> Result<(String, Option<String>), String> {
    let (cid, sec) = get_oauth_creds();
    let resp = ureq::post(TOKEN_ENDPOINT)
        .send_form(&[
            ("code", code),
            ("client_id", &cid),
            ("client_secret", &sec),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ])
        .map_err(|e| format!("Token exchange failed: {e}"))?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("Invalid token response: {e}"))?;

    let acc = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing access_token".to_string())?
        .to_string();

    let ref_tok = json
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    Ok((acc, ref_tok))
}

pub(crate) fn spawn_antigravity_worker(ctx: egui::Context, state: Arc<Mutex<AntigravityState>>) {
    thread::spawn(move || {
        let mut last_fetch = Instant::now() - Duration::from_secs(600);

        loop {
            let (do_login, do_logout, do_refresh) = {
                let mut st = state.lock().unwrap();
                let l = st.trigger_login;
                let o = st.trigger_logout;
                let r = st.trigger_refresh;
                st.trigger_login = false;
                st.trigger_logout = false;
                st.trigger_refresh = false;
                (l, o, r)
            };

            if do_logout {
                if let Some(p) = token_cache_path() {
                    let _ = fs::remove_file(p);
                }
                let mut st = state.lock().unwrap();
                st.status = AuthStatus::LoggedOut;
                st.quota = None;
                ctx.request_repaint();
            }

            if do_login {
                perform_oauth_flow(&ctx, &state);
            }

            // Periodic or manual fetch
            if do_refresh || last_fetch.elapsed() > Duration::from_secs(60) {
                last_fetch = Instant::now();

                let email = detect_logged_in_email()
                    .unwrap_or_else(|| "luthfikamal13@gmail.com".to_string());

                {
                    let mut st = state.lock().unwrap();
                    st.is_loading = true;
                }
                ctx.request_repaint();

                match fetch_quota_from_agy(&email) {
                    Ok(quota) => {
                        let mut st = state.lock().unwrap();
                        st.status = AuthStatus::LoggedIn;
                        st.quota = Some(quota);
                        st.last_refresh = Some(Local::now().naive_local());
                        st.is_loading = false;
                        ctx.request_repaint();
                    }
                    Err(e) => {
                        let mut st = state.lock().unwrap();
                        st.is_loading = false;
                        if e.contains("Not logged in") {
                            st.status = AuthStatus::LoggedOut;
                        } else if st.quota.is_none() {
                            st.status = AuthStatus::Error(e);
                        }
                        ctx.request_repaint();
                    }
                }
            }

            thread::sleep(Duration::from_millis(250));
        }
    });
}

fn perform_oauth_flow(ctx: &egui::Context, state: &Arc<Mutex<AntigravityState>>) {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(e) => {
            let mut st = state.lock().unwrap();
            st.status = AuthStatus::Error(format!("Failed to bind local server: {e}"));
            ctx.request_repaint();
            return;
        }
    };

    let port = match listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(e) => {
            let mut st = state.lock().unwrap();
            st.status = AuthStatus::Error(format!("Failed to get local port: {e}"));
            ctx.request_repaint();
            return;
        }
    };

    {
        let mut st = state.lock().unwrap();
        st.status = AuthStatus::Authenticating { port };
    }
    ctx.request_repaint();

    let redirect_uri = format!("http://127.0.0.1:{port}");
    let enc_redirect = format!("http%3A%2F%2F127.0.0.1%3A{port}");
    let (client_id, _) = get_oauth_creds();

    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={client_id}&response_type=code&scope={SCOPES}&redirect_uri={enc_redirect}&access_type=offline&prompt=consent"
    );

    // Launch default browser
    let _ = std::process::Command::new("cmd")
        .args(["/c", "start", "", &auth_url])
        .spawn();

    let _ = listener.set_nonblocking(false);

    match listener.incoming().next() {
        Some(Ok(mut stream)) => {
            let mut buf = [0u8; 4096];
            let read_bytes = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..read_bytes]);

            let mut code = None;
            if let Some(pos) = req.find("code=") {
                let rest = &req[pos + 5..];
                let end = rest
                    .find('&')
                    .or_else(|| rest.find(' '))
                    .unwrap_or(rest.len());
                let raw_code = &rest[..end];
                code = Some(raw_code.to_string());
            }

            if let Some(auth_code) = code {
                let html = r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>EmelkaNotch - Antigravity Auth</title>
<style>
  body { background: #0b0c10; color: #e8eaf0; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; display: flex; flex-direction: column; align-items: center; justify-content: center; height: 85vh; margin: 0; }
  .card { background: #171922; border: 1px solid #2a2c34; border-radius: 16px; padding: 32px 48px; text-align: center; box-shadow: 0 10px 30px rgba(0,0,0,0.5); }
  h2 { color: #78aaff; margin-top: 0; font-size: 24px; }
  p { color: #8c92a0; font-size: 15px; margin-bottom: 0; }
</style>
</head>
<body>
  <div class="card">
    <h2>&#10003; Antigravity Connected</h2>
    <p>Authentication successful! You can now close this tab and return to EmelkaNotch.</p>
  </div>
</body>
</html>"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html.len(),
                    html
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();

                match exchange_code_for_token(&auth_code, &redirect_uri) {
                    Ok((acc, ref_tok)) => {
                        if let Some(p) = token_cache_path() {
                            let mut map = serde_json::Map::new();
                            map.insert("access_token".to_string(), serde_json::Value::String(acc));
                            if let Some(rf) = ref_tok {
                                map.insert(
                                    "refresh_token".to_string(),
                                    serde_json::Value::String(rf),
                                );
                            }
                            let _ = fs::write(p, serde_json::Value::Object(map).to_string());
                        }

                        let email = detect_logged_in_email()
                            .unwrap_or_else(|| "luthfikamal13@gmail.com".to_string());

                        if let Ok(quota) = fetch_quota_from_agy(&email) {
                            let mut st = state.lock().unwrap();
                            st.status = AuthStatus::LoggedIn;
                            st.quota = Some(quota);
                            st.last_refresh = Some(Local::now().naive_local());
                        } else {
                            let mut st = state.lock().unwrap();
                            st.status = AuthStatus::LoggedIn;
                        }
                    }
                    Err(e) => {
                        let mut st = state.lock().unwrap();
                        st.status = AuthStatus::Error(format!("OAuth exchange failed: {e}"));
                    }
                }
            } else {
                let err_resp =
                    "HTTP/1.1 400 Bad Request\r\nContent-Length: 17\r\n\r\nMissing auth code";
                let _ = stream.write_all(err_resp.as_bytes());
                let mut st = state.lock().unwrap();
                st.status = AuthStatus::Error("No authorization code received".to_string());
            }
        }
        _ => {
            let mut st = state.lock().unwrap();
            st.status = AuthStatus::Error("OAuth server connection timed out".to_string());
        }
    }
    ctx.request_repaint();
}
