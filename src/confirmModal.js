const { invoke, appWindow } = window.__TAURI__.tauri;
const {emit, listen} = window.__TAURI__.event;

let settingId;
let remainingTime;

async function init(){
    const modal = document.querySelector('.modal-content');
    const cancelBtn = document.getElementById('cancelBtn');
    const confirmBtn = document.getElementById('confirmBtn');

    const params = new URLSearchParams(window.location.search);
    settingId = params.get('key');

    invoke('get_change_status', { settingId })
        .then(statusChange => {
            console.log("statusChange:", statusChange);
            if(!statusChange.isChanging){
                return;
            }
            const delayTime = statusChange.delayTimeOutAtTimeOfChange || statusChange.currentTimeout || 180000;
            showProgressBar(statusChange.timeRemaining, delayTime);
        });


    if(cancelBtn){
        cancelBtn.addEventListener('click', () => {
            if(modal && modal.style && modal.style.display){
                modal.style.display = 'none';
            }
            closeModal();
        });
    }

    if(confirmBtn){
        confirmBtn.addEventListener('click', () => {
            if(modal){
                modal.style.display = 'none';
            }

            invoke('start_countdown_timer', {settingId})
                .then(() => {
                    console.log("Successfully started a timer");
                    closeModal();
                });
        });
    }
}

init();

function closeModal(){
    invoke('close_invoking_window');
}

function showProgressBar(timeRemaining, delayTimeOutAtTimeOfChange){
    const confirmDialog = document.getElementById("confirm-modal");
    const progressContainer = document.getElementById('progressContainer');
    const timeEl = document.getElementById('timeRemaining');
    const barEl = document.getElementById('progressBar');
    const cancelBtnForProgressBar = document.getElementById('cancelBtnForProgressBar');
    const confirmBtn = document.getElementById('confirmBtn');

    if (!confirmDialog || !progressContainer || !timeEl || !barEl) {
        console.warn('showProgressBar: missing DOM elements');
        return;
    }

    confirmDialog.style.display = 'none';
    progressContainer.style.display = 'block';

    const total = Number(delayTimeOutAtTimeOfChange) || Number(timeRemaining) || 1;
    const endTime = Date.now() + (Number(timeRemaining) || 0);

    if (cancelBtnForProgressBar) {
        cancelBtnForProgressBar.addEventListener('click', () => {
            if (confirmBtn) confirmBtn.disabled = true;
            invoke('cancel_countdown_timer', { settingId }).then(() => closeModal());
        }, { once: true });
    }

    let interval = null;

    const updateProgress = () => {
        const remaining = Math.max(endTime - Date.now(), 0);

        const percentagePassed = (remaining / delayTimeOutAtTimeOfChange);
        console.log(percentagePassed);
        const percent = 100 - Math.floor(percentagePassed * 100);

        timeEl.innerText = `${Math.ceil(remaining / 1000)}s`;
        barEl.style.width = `${percent}%`;

        if (remaining <= 0) {
            if (interval) clearInterval(interval);
            progressContainer.style.display = 'none';
            confirmDialog.style.display = 'block';
            if (confirmBtn) confirmBtn.disabled = true;
            closeModal();
        }
    };

    updateProgress();
    interval = setInterval(updateProgress, 1000);
}