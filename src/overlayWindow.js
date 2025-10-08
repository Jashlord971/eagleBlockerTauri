const { listen, emit } = window.__TAURI__.event;
const { invoke } = window.__TAURI__.tauri;
let appInfo;

document.addEventListener('DOMContentLoaded', async () => {
    initializeCloseButton();
    setUpAppInfo();
    hideElement('hidden');
    setUpAltCloseButton();
});

function setUpAppInfo(){
    const params = new URLSearchParams(window.location.search);
    appInfo = {
        displayName: params.get('displayName') || '',
        processName: params.get('processName') || params.get('procName') || params.get('process') || ''
    };

    const name = appInfo.displayName || appInfo.processName || 'the application';
    const nameEl = document.getElementById('appName');
    if (nameEl) nameEl.textContent = name;

    const rawProc = (appInfo.processName || '').trim();
    const processNameLower = rawProc.toLowerCase();
    const processFile = processNameLower.split('\\').pop().split('/').pop();

    const systemApps = new Set(['taskmgr.exe', 'mmc.exe', 'control.exe', 'systemsettings.exe']);
    const isSystem = systemApps.has(processFile) || systemApps.has(processNameLower) || /\\system32\\taskmgr\.exe$/i.test(rawProc);

    if (isSystem) {
        const paragraph = document.getElementById('warning-paragraph');
        if (paragraph) {
            paragraph.textContent =
                "We noticed a protected system app is open (Task Manager, Task Scheduler, or Control Panel). " +
                "Please close that window from the taskbar to allow this overlay to close automatically.";
        }

        const closeButton = document.getElementById('closeBtn');
        if (closeButton) closeButton.style.display = 'none';

        showElement('hidden');
        const alt = document.getElementById('altButton');
        if (alt) alt.style.display = 'inline-block';
    }

    console.log('overlay appInfo:', appInfo, { processFile, isSystem });
}

function setUpAltCloseButton() {
    const altButton = document.getElementById("altButton");
    if (!altButton) return;

    altButton.addEventListener("click", () => {
        closeOverlay();
    });
}

function showElement(id) {
    const el = document.getElementById(id);
    if (el) {
        el.style.display = "block";
    }
}

function hideElement(id) {
    const el = document.getElementById(id);
    if (el) {
        el.style.display = "none";
    }
}

function closeOverlay(){
    invoke('close_overlay_window');
}

function initializeCloseButton(){
    const closeButton = document.getElementById('closeBtn');
    if(!closeButton){
        return;
    }

    closeButton.addEventListener('click', () => {
        invoke('close_app', { processName: appInfo.processName })
            .then(result => {
                console.log("should close the overlay window");
                if(result){
                    closeOverlay();
                }
            })
            .catch(err => console.error('close_app error', err));

        const loadingIcon = document.getElementById('loading');
        if(loadingIcon){
            loadingIcon.style.display = 'block';
        }

        closeButton.disabled = true;
        closeButton.textContent = "Closing...";

        setTimeout(() => {
            showElement('hidden');
            hideElement('shown');
        }, 60_000);
    });
}