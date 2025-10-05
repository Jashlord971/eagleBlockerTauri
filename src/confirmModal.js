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
            showProgressBar(statusChange.timeRemaining, statusChange.delayTimeOutAtTimeOfChange);
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
    invoke('close_confirm_modal');
}

function showProgressBar(timeRemaining, delayTimeOutAtTimeOfChange){
    const confirmDialog = document.getElementById("confirm-modal");
    const progressContainer = document.getElementById('progressContainer');

    confirmDialog.style.display = 'none';
    progressContainer.style.display = 'block';

    const cancelBtnForProgressBar = document.getElementById('cancelBtnForProgressBar');
    cancelBtnForProgressBar.addEventListener('click', () => {
        confirmBtn.disabled = true;
        closeModal();
        invoke('cancel_countdown_timer', {settingId});
    });

    const endTime = Date.now() + timeRemaining;

    const updateProgress = () => {
        const remaining = Math.max(endTime - Date.now(), 0);
        const percent = 100 - Math.floor((remaining / delayTimeOutAtTimeOfChange) * 100);

        document.getElementById('timeRemaining').innerText = `${Math.ceil(remaining / 1000)}s`;
        document.getElementById('progressBar').style.width = `${percent}%`;

        if (remaining <= 0) {
            clearInterval(interval);
            progressContainer.style.display = 'none';
            confirmDialog.style.display = 'block';

            const confirmBtn = document.getElementById('confirmBtn');
            confirmBtn.disabled = true;
            closeModal();
        }
    };

    updateProgress();
    const interval = setInterval(updateProgress, 1000);
}