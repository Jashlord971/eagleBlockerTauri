const { invoke } = window.__TAURI__.tauri;

const radioButtons = document.querySelectorAll('input[name="approvalOption"]');
const modal = document.querySelector('.modal-content');
const cancelBtn = document.getElementById('cancelBtn');
const confirmBtn = document.getElementById('confirmBtn');

cancelBtn.addEventListener('click', () => modal.style.display = 'none');

radioButtons.forEach(radio => {
    radio.addEventListener('change', (event) => confirmBtn.disabled = !event.target.checked);
});

const turnOnDNS = (isStrict) => invoke('turn_on_dns', {isStrict});

confirmBtn.addEventListener('click', async () => {
    const selectedOption = document.querySelector('input[name="approvalOption"]:checked').value;
    const isStrict = (selectedOption === "strict");
    await turnOnDNS(isStrict);
    modal.style.display = 'none';
});