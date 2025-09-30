const { invoke } = window.__TAURI__.tauri;
const settingId = 'delayTimeOut';

async function validateForm() {
    return shouldDisableConfirmButton()
        .then(result => {
            const confirmBtn = document.getElementById('confirmBtn');
            confirmBtn.disabled = result;
        })
        .catch(error => console.log(error));
}

function getDelayTimeout(){
    return invoke('get_delay_time_out');
}

async function getCurrentDelayTimeout(){
    const defaultDelay = 180000;
    return getDelayTimeout()
            .then(delayTimeout => delayTimeout)
            .catch(error => {
                console.error("Encountered an error when getting the current delayTimeOut: " + error.message);
                return defaultDelay;
            });
}

function shouldDisableConfirmButton() {
    const selected = document.querySelector('input[name="delay"]:checked');
    if (!selected) {
        return Promise.resolve(true);
    }

    return getCurrentDelayTimeout()
        .then(delayTimeoutValue => {
            const value = selected['value'];

            if (value !== 'custom') {
                try {
                    const currentValue = eval(value);
                    return currentValue === delayTimeoutValue;
                } catch {
                    return false;
                }
            }

            const customDays = parseFloat(document.getElementById('customDays')['value']);
            if (isNaN(customDays) || customDays <= 0) {
                return true;
            }
            const millis = customDays * 24 * 60 * 60 * 1000;
            return millis === delayTimeoutValue;
        })
        .catch(error => {
            console.log("Encountered error while disabling confirmation button");
            console.log(error);
            return false;
        });
}


function getSelectedValue(selectedValues){
    if(!selectedValues){
        return null;
    }
    const selectedValue = eval(selectedValues['value']);
    if (selectedValue === 'custom') {
        const customDays = document.getElementById('customDays');
        const days = parseFloat(customDays['value']);
        if (isNaN(days) || days <= 0) {
            alert("Please enter a valid custom delay in days.");
            return;
        }
        return days * 24 * 60 * 60 * 1000;
    }
    else {
        return parseInt(selectedValue);
    }
}

function initializeConfirmButton(confirmBtn){
    if(!confirmBtn){
        return;
    }

    confirmBtn.addEventListener('click', async () => {
        const selector = 'input[name="delay"]:checked';
        const selectedValues = document.querySelector(selector);
        const selectedValue = getSelectedValue(selectedValues);

        return getCurrentDelayTimeout()
            .then(delayTimeoutValue => {
                if(selectedValue === delayTimeoutValue){
                    return;
                }
                if (selectedValue > delayTimeoutValue) {
                    setDelayTimeOut(selectedValue)
                        .then(() => {
                            confirmBtn.disabled = true;
                            location.reload();
                        });
                }
                else {
                    const payload = {
                        settingId,
                        remainingTime : delayTimeoutValue,
                        targetTimeout: selectedValue
                    };
                    invoke('start_countdown_timer', payload);
                    showProgressBar(delayTimeoutValue, delayTimeoutValue);
                }
            })
            .catch(error => {
                console.log("Encountered error while disabling confirmation button");
                console.log(error);
            });
    });
}

function setDelayTimeOut(selectedValue){
    return invoke('save_preference', {key: settingId, value: selectedValue});
}

function initializeCancelButton(confirmBtn, progressContainer, delayTimesDialog){
    const cancelButton = document.getElementById("cancelBtn");
    cancelButton.addEventListener('click', () => {
        confirmBtn.disabled = true;
        progressContainer.style.display = 'none';
        delayTimesDialog.style.display = 'block';
        invoke('cancel_countdown_timer', {settingId});
    });
}

function showProgressBar(timeRemaining, currentTimeout){
    const delayTimesDialog = document.getElementById("delayTimes");
    const progressContainer = document.getElementById('progressContainer');

    delayTimesDialog.style.display = 'none';
    progressContainer.style.display = 'block';

    const endTime = Date.now() + timeRemaining;

    const updateProgress = () => {
        const remaining = Math.max(endTime - Date.now(), 0);
        const percent = 100 - Math.floor((remaining / currentTimeout) * 100);

        document.getElementById('timeRemaining').innerText = `${Math.ceil(remaining / 1000)}s`;
        document.getElementById('progressBar').style.width = `${percent}%`;

        if (remaining <= 0) {
            clearInterval(interval);
            progressContainer.style.display = 'none';
            delayTimesDialog.style.display = 'block';

            const confirmBtn = document.getElementById('confirmBtn');
            confirmBtn.disabled = true;
        }
    };

    updateProgress();
    const interval = setInterval(updateProgress, 1000);
}

async function init(){
    const customRadio = document.getElementById('customRadio');
    const customInput = document.getElementById('customInput');

    const confirmBtn = document.getElementById('confirmBtn');
    const delayTimesDialog = document.getElementById("delayTimes");
    const progressContainer = document.getElementById('progressContainer');

    initializeConfirmButton(confirmBtn);
    initializeCancelButton(confirmBtn, progressContainer, delayTimesDialog);

    const customDays = document.getElementById('customDays');
    customDays.addEventListener('input', validateForm);

    document.querySelectorAll('input[name="delay"]').forEach(radio => {
        radio.addEventListener('change', () => {
            customInput.style.display = customRadio['checked'] ? 'block' : 'none';
            void validateForm();
        });
    });

    setCurrentDelayTimeout(customInput, customDays, customRadio)
        .then(() => console.log("Successfully set current delay timeout"))
        .catch(error => console.log(error));

    invoke('get_delay_change_status')
        .then(status => {
            if (status && status.isChanging) {
                showProgressBar(status.timeRemaining, status.delayTimeOutAtTimeOfChange);
            }
        })
        .catch(error => {
            console.log("Encountered error while getting delay change status");
            console.log(error);
        });

    void validateForm();
}

function setCurrentDelayTimeout(customInput, customDays, customRadio){
    return getCurrentDelayTimeout()
        .then(delayTimeoutValue => {
            if (isNaN(delayTimeoutValue)){
                return;
            }

            const radioButtons = document.querySelectorAll('input[name="delay"]');

            const matched = Array.from(radioButtons)
                .filter(radio => radio && radio['value'] && radio['value'] !== 'custom')
                .find(radio => {
                    try{
                        const value = eval(radio['value']);
                        return value === delayTimeoutValue;
                    } catch(error){
                        return false;
                    }
                });

            if(matched){
                matched.checked = true;
                customInput.style.display = 'none';
            } else {
                customRadio.checked = true;
                customInput.style.display = 'block';
                const days = delayTimeoutValue / (24 * 60 * 60 * 1000);
                customDays.value = days.toFixed(2);
            }
        })
        .catch(error => console.log(error));
}

void init();