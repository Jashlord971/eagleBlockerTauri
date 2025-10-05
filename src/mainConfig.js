const { invoke } = window.__TAURI__.tauri;
const { listen, emit } = window.__TAURI__.event;
const { WebviewWindow } = window.__TAURI__.window;

let overlayWindow = null;
let confirmDialog = null;

window.addEventListener('DOMContentLoaded', () => init());

function init(){
    initializeOverlaySwitch();
    initializeEnableProtectiveDns();
    initializeSettingsAndAppProtection();
    initializeSafeSearchProtection();
    initializeFlagListener();
    setUpModalCloser();
    listenForRefresh();
    listenForCloseOfOverlayWindow();
}

function getPreference(key){
    return invoke("read_preference", {key});
}

function saveSharedPreference(key, value = true){
    const args = { key, value };
    return invoke("save_preference", args).then(() => console.log("Preference saved"));
}

function openConfirmationDialog(key){
    const config = {
        url: `confirmDialog.html?key=${encodeURIComponent(key)}`,
        title: 'Configuration Change',
        width: 450,
        height: 250,
        alwaysOnTop: true,
        focus: true
    };
    confirmDialog = new WebviewWindow('confirmDialog', config);
}

function openDnsStrictnessLevelDialog(){
    const label = `dnsConfirmationModal-${Date.now()}`;

    const config = {
        url: 'dnsConfirmationModal.html',
        title: 'Choose DNS strictness level',
        width: 450,
        height: 250
    };

    const dnsDialog = new WebviewWindow(label, config);
    dnsDialog.once('tauri://created', () => console.log("DNS dialog created"));
    dnsDialog.once('tauri://error', (e) => console.error("Failed to create DNS dialog:", e));
}

async function initializeOverlaySwitch() {
    const key = 'overlayRestrictedContent';
    const overlaySwitch = document.getElementById(key);
    if (!overlaySwitch){
        return;
    }

    const isChecked = await getPreference(key);
    overlaySwitch.checked = isChecked;
  
    overlaySwitch.addEventListener('change', async () => {
        const value = await getPreference(key);
        if (value) {
            overlaySwitch.checked = true;
            openConfirmationDialog(key);
        }
        else {
            saveSharedPreference(key);
        } 
    });
}

async function isProtectiveDnsOn(key){
    const valueInDb = await getPreference(key);
    const isDnsMadeSafe = await invoke("is_dns_made_safe");
    return valueInDb && isDnsMadeSafe;
}

async function initializeEnableProtectiveDns(){
    const key = "enableProtectiveDNS";
    const protectiveDnsSwitch = document.getElementById(key);

    if(!protectiveDnsSwitch){
        return;
    }

    const isProtectiveDnsActive = await isProtectiveDnsOn(key);
    protectiveDnsSwitch.checked = isProtectiveDnsActive;

    protectiveDnsSwitch.addEventListener('change', async () => {
        const isProtectiveDnsActiveNow = await isProtectiveDnsOn(key);
        if(isProtectiveDnsActiveNow){
            protectiveDnsSwitch.checked = true;
            openConfirmationDialog(key);
        }
        else{
            protectiveDnsSwitch.checked = false;
            openDnsStrictnessLevelDialog();
        }
    });
}

async function initializeSettingsAndAppProtection(){
    const key = 'blockSettingsSwitch';
    const settingsAndAppProtectionSwitch = document.getElementById(key);

    if(!settingsAndAppProtectionSwitch){
        return;
    }

    const isChecked = await getPreference(key);
    if(isChecked){
        invoke('turn_on_settings_and_app_protection');
    } else{
        invoke('stop_settings_and_app_protection');
    }

    settingsAndAppProtectionSwitch.checked = isChecked;

    settingsAndAppProtectionSwitch.addEventListener('change', async () => {
        const isSettingsProtectionActive = await getPreference(key);
        if(isSettingsProtectionActive){
            settingsAndAppProtectionSwitch.checked = true;
            openConfirmationDialog(key);
        }
        else{
            saveSharedPreference(key);
        }
    });
}

async function initializeSafeSearchProtection(){
    const key = "enforceSafeSearch";
    const safeSearchSwitch = document.getElementById(key);
    if(!safeSearchSwitch){
        return;
    }

    const isChecked = await getPreference(key) && isSafeSearchEnabled();
    safeSearchSwitch.checked = isChecked;

    safeSearchSwitch.addEventListener('change', async () => {
        const isSafeSearchEnabledLocally = await isSafeSearchEnabled();
        const isSafeSearchEnabledValue = await getPreference(key) && isSafeSearchEnabledLocally;
        if(isSafeSearchEnabledValue){
            safeSearchSwitch.checked = true;
            openConfirmationDialog(key);
        }
        else if(isSafeSearchEnabledLocally){
            saveSharedPreference(key);
        }
        else{
            invoke('enable_safe_search');
        }
    });
}

function isSafeSearchEnabled(){
    return invoke('is_safe_search_enabled');
}

function showOverlay(displayName = '', processName = '') {
  if (overlayWindow !== null) return;

  const label = `overlay-${Date.now()}`;

  const url = `overlayWindow.html?displayName=${encodeURIComponent(displayName)}&processName=${encodeURIComponent(processName)}`;
  overlayWindow = new WebviewWindow(label, {
    url,
    title: 'Overlay',
    //fullscreen: true,
    decorations: false,
    transparent: false,
    alwaysOnTop: true,
    width: 800,
    height: 600
  });

  overlayWindow.once('tauri://destroyed', () => overlayWindow = null);

  overlayWindow.once('tauri://created', () => {
    overlayWindow.emit('appInfo', { displayName, processName });
    console.log({displayName, processName});
  });
}

function closeOverlay() {
  if (!overlayWindow) return;
  overlayWindow.close();
  overlayWindow = null;
}

function initializeFlagListener(){
    return listen('flag-app-with-overlay', (event) => {
        const { displayName, processName } = event.payload ?? {};
        console.log('flagged process:', displayName, processName);
        showOverlay(displayName, processName);
    });
}

function closeConfirmDialog(){
    if(confirmDialog){
        confirmDialog.close();
        confirmDialog = null;
    }
}

function setUpModalCloser(){
    return listen('close_confirm_modal_prompt', (event) => {
        console.log("here");
        closeConfirmDialog();
    });
}

function listenForRefresh(){
    return listen('preferences-updated', () => {
        closeConfirmDialog();
        window.location.reload();
    });
}

function listenForCloseOfOverlayWindow(){
    listen('close_overlay_window_prompted', () => closeOverlay());
}

