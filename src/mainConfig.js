const { invoke } = window.__TAURI__.tauri;
const { listen, emit } = window.__TAURI__.event;
const { WebviewWindow } = window.__TAURI__.window;

let overlayWindow = null;
let confirmDialog = null;
let dnsDialog = null;

let tooltipsInitialized = false;

const KEYS = [
    'overlayRestrictedContent',
    "enableProtectiveDNS",
    'blockSettingsSwitch',
    "enforceSafeSearch"
];

function initTooltips() {
    if (tooltipsInitialized) {
        return;
    }
    tooltipsInitialized = true;

    let tip = document.querySelector('.tooltip-box');
    if (!tip) {
        tip = document.createElement('div');
        tip.className = 'tooltip-box';
        document.body.appendChild(tip);
    }

    const show = (el, e) => {
        const text = el.getAttribute('data-description');
        if (!text) return;
        tip.textContent = text;
        tip.style.display = 'block';
        position(e);
    };

    const hide = () => {
        tip.style.display = 'none';
    };

    const position = (e) => {
        const pad = 12;
        const vw = window.innerWidth;
        const vh = window.innerHeight;
        const rect = tip.getBoundingClientRect();
        let x = e.clientX + pad;
        let y = e.clientY + pad;
        if (x + rect.width > vw) x = Math.max(0, vw - rect.width - pad);
        if (y + rect.height > vh) y = Math.max(0, vh - rect.height - pad);
        tip.style.left = `${x + window.scrollX}px`;
        tip.style.top = `${y + window.scrollY}px`;
        tip.style.position = 'absolute';
    };

    const bind = (el) => {
        el.addEventListener('mouseenter', (e) => show(el, e));
        el.addEventListener('mousemove', position);
        el.addEventListener('mouseleave', hide);
    };

    document.querySelectorAll('.info-icon, .timer-icon').forEach(bind);
}

window.addEventListener('DOMContentLoaded', () => init());

function init(){
    initializeOverlaySwitch();
    initializeEnableProtectiveDns();
    initializeSettingsAndAppProtection();
    initializeSafeSearchProtection();
    listenForRefresh();
    listenForTimerUpdate();
    initTooltips();

    Array.from(KEYS).map(key => showTimerIcon(key));
}

function showTimerIcon(key){
    return invoke('get_change_status', {settingId: key})
        .then(status => {
            if(status && status.isChanging){
                const timerIconKey = "timer-" + key;
                showElement(timerIconKey);
            }
        });
}

function getPreference(key){
    return invoke("read_preference", {key});
}

function saveSharedPreference(key, value = true){
    const args = { key, value };
    return invoke("save_preference", args).then(() => console.log("Preference saved"));
}

function openConfirmationDialog(key){
    if (!key) return;

    if (confirmDialog) {
        try {
            confirmDialog.show();
            confirmDialog.setFocus();
            confirmDialog.emit('update-confirm-key', { key });
        } catch (e) {
            console.warn('openConfirmationDialog: existing dialog focus/emit failed, recreating', e);
            confirmDialog = null;
        }
    }

    if (!confirmDialog) {
        const url = `confirmDialog.html?key=${encodeURIComponent(key)}`;
        const config = {
            url,
            title: 'Delay Status',
            width: 450,
            height: 250,
            alwaysOnTop: true,
            focus: true
        };
        confirmDialog = new WebviewWindow('confirmDialog', config);
        confirmDialog.once('tauri://destroyed', () => { confirmDialog = null; });
    }
}

function openDnsStrictnessLevelDialog(){
    invoke("show_dns_confirmation_modal");
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

function showElement(id) {
    const el = document.getElementById(id);
    if (el) {
        el.classList.remove('is-hidden');
    }
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

function listenForRefresh(){
    return listen('preferences-updated', () => {
        invoke('close_confirmation_dialog');
        window.location.reload();
    });
}

function listenForTimerUpdate(){
    return listen("timer-updated", () => window.location.reload());
}