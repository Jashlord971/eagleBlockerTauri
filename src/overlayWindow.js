const { listen, emit } = window.__TAURI__.event;
const { invoke } = window.__TAURI__.tauri;
let appInfo;

document.addEventListener('DOMContentLoaded', async () => {
    initializeCloseButton();
    setUpAppInfo();
    hideElement('hidden');
});

function hideButton(){
    const closeButton = document.getElementById('closeBtn');
    if (closeButton) closeButton.style.display = 'none';

    showElement('hidden');
    const alt = document.getElementById('altButton');
    if (alt) alt.style.display = 'inline-block';
}

function setUpAppInfo(){
    const params = new URLSearchParams(window.location.search);
    appInfo = {
        code: params.get('code') || '',
        displayName: params.get('displayName') || '',
        processName: params.get('processName') || params.get('procName') || params.get('process') || ''
    };

    const name = appInfo.displayName || appInfo.processName || 'the application';
    const nameEl = document.getElementById('appName');
    if (nameEl) nameEl.textContent = name;

    const rawProc = (appInfo.processName || '').trim();
    const processNameLower = rawProc.toLowerCase();
    const processFile = processNameLower.split('\\').pop().split('/').pop();

    const code = appInfo.code || '';
    const paragraph = document.getElementById('warning-paragraph');

    if (code === 'protected-system-app') {
        paragraph.textContent =
                "We noticed a protected system app is open (Task Manager, Task Scheduler, or Control Panel). " +
                "Please close that window from the taskbar to allow this overlay to close automatically.";

        hideButton();
    }
    else if(code === 'browser-with-proxy'){
        paragraph.textContent =
                "We noticed a browser application running while TOR, a VPN, or Proxy is active on your system. " +
                "For your safety, please close the browser to allow this overlay to close automatically.";
        hideButton();
    }
    else if(code === "uninstaller-window-detected"){
        paragraph.textContent =
                "We detected that the Eagle Blocker uninstaller window is open. " +
                "Please close the uninstaller window to allow this overlay to close automatically.";

        hideButton();
    }
    else if(code == "unsupported_browser"){
        paragraph.textContent =
                "We detected that you are using an unsupported browser. " +
                "Please close the browser to allow this overlay to close automatically.";
    }
    else if(code === "browser-with-vpn"){
        paragraph.textContent =
                "We noticed a supported browser with a vpn extension running. " +
                "For your safety, please close the browser to allow this overlay to close automatically.";
    }

    const isSystem = code === 'protected-system-app';
    console.log('overlay appInfo:', appInfo, { processFile, isSystem });
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
        invoke('close_app', { processName: appInfo.processName });

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