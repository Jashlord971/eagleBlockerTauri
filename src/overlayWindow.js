const { listen, emit } = window.__TAURI__.event;
const { invoke } = window.__TAURI__.tauri;
let appInfo;

document.addEventListener('DOMContentLoaded', async () => {
    setUpAppInfo();
    initializeCloseButton();
    hideElement('hidden');
    setUpAltCloseButton();
});

function setUpAppInfo(){
    const params = new URLSearchParams(window.location.search);
    appInfo = {
        displayName: params.get('displayName') || '',
        processName: params.get('processName') || ''
    };
    
    const name = appInfo.displayName || appInfo.processName || 'the application';
    document.getElementById('appName').textContent = name;
    console.log(appInfo);
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
    emit('overlay-request-close');
}

function initializeCloseButton(){
    const closeButton = document.getElementById('closeBtn');
    if(!closeButton){
        return;
    }

    closeButton.addEventListener('click', () => {
        if(appInfo.processName && appInfo.processName.toLowerCase() === 'taskmgr.exe'){
            // TODO: show message to ask user to self close system apps
            return;
        }

        invoke('close_app', { processName: appInfo.processName })
            .then(result => {
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