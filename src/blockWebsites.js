const { invoke } = window.__TAURI__.tauri;
const { listen } = window.__TAURI__.event;
const { WebviewWindow } = window.__TAURI__.window;
const HOSTS_FILE = 'C:\\Windows\\System32\\drivers\\etc\\hosts';

let confirmDialog;

let list;
const setOfEntries = new Set();

function init(){
    if(!window){
        return;
    }

    initializeListener();
    listenForPrimeDeletion();

    window.addEventListener('DOMContentLoaded', async () => {
        const modal = document.getElementById('modal');
        const input = document.getElementById('modal-input');
        const saveBtn = document.getElementById('save-btn');
        const cancelBtn = document.getElementById('cancel-btn');
        const addBtn = document.getElementById('add-btn');

        addBtn.addEventListener('click', () => {
            input.value = '';
            modal.style.display = 'flex';
            input.focus();
        });

        cancelBtn.addEventListener('click', () => closeModal(modal));

        saveBtn.addEventListener('click', async () => {
            const value = input.value.trim();
            if (!value) {
                return;
            }

            return addToBlockListsForWebsites(modal, value)
                .then(success => {
                    if (success) {
                        window.location.reload();
                        modal.style.display = 'none';
                    }
                })
                .catch(error => {
                    console.error('Error blocking website:', error);
                    alert(`Failed to block website: ${error.message}`);
                });
        });

        await renderTable();
    });

    window.removeItem = (index) => removeWebsite(index);
}

function removeWebsite(index){
    if(!list || index < 0 || index > list.length){
        return;
    }
    const site = list[index];
    invoke('remove_block_website', {site})
        .then(() => {
            window.location.reload();
            setOfEntries.delete(item);
        })
        .catch(error => console.log(error));
}

function initializeListener(){
    listen("block-data-updated").then(() => renderTable());
}

init();

function createEmptyRow(message) {
    const row = document.createElement('tr');
    const cell = document.createElement('td');
    cell.colSpan = 2;
    cell.textContent = message;
    row.appendChild(cell);
    return row;
}

function createDataRow(item, index, isAllowedToDelete) {
    const row = document.createElement('tr');

    const nameCell = document.createElement('td');
    nameCell.textContent = typeof item === 'string' ? item : item.displayName;

    const deleteCell = document.createElement('td');
    const deleteButton = createDeleteButton(item, index, isAllowedToDelete);
    deleteCell.appendChild(deleteButton);

    row.appendChild(nameCell);
    row.appendChild(deleteCell);

    return row;
}

function createDeleteButton(item, index, isAllowedToDelete) {
    const button = document.createElement('button');
    const { label, color } = getButtonTextAndColour(isAllowedToDelete);

    button.textContent = label;
    button.style.backgroundColor = '#FF5555';
    button.style.color = color;
    button.style.border = 'none';
    button.style.padding = '5px 10px';
    button.style.borderRadius = '5px';
    button.style.cursor = 'pointer';

    button.onclick = () => {
        if (isAllowedToDelete) {
            removeItem(index);
        } else {
            const key = "allowedForUnblockWebsites-->" + item;
            invoke("get_change_status", {settingId: key})
                .then(statusChange => {
                    if(statusChange.isChanging){
                        openConfirmationDialog(key);
                    }
                    else{
                        const payload = {
                            itemType: "website",
                            name: item
                        }
                        invoke("prime_for_deletion", payload);
                    }
                });
        }
    };

    return button;
}


function sleep(ms) {
    return new Promise((res) => setTimeout(res, ms));
}

async function fetchBlockDataWithRetry(attempts = 5, baseDelay = 100) {
    for (let i = 0; i < attempts; i++) {
        try {
            const blockData = await invoke('get_block_data');
            // basic validation: must be an object and contain blockedWebsites array (could legitimately be empty)
            if (
                blockData &&
                typeof blockData === 'object' &&
                (Array.isArray(blockData.blockedWebsites) || Object.prototype.hasOwnProperty.call(blockData, 'blockedWebsites'))
            ) {
                return blockData;
            }
        } catch (err) {
            console.warn('fetchBlockDataWithRetry: invoke failed, attempt', i + 1, err);
        }
        await sleep(baseDelay * Math.pow(2, i));
    }
    
    return invoke('get_block_data').catch((e) => {
        console.error('fetchBlockDataWithRetry: final attempt failed', e);
        return null;
    });
}

async function renderTable() {
    const tbody = document.querySelector('#data-table tbody');
    tbody.innerHTML = '';

    try{
        const blockData =  await fetchBlockDataWithRetry(5, 100);
        const blockedWebsites = new Set(Array.from(blockData?.blockedWebsites || []));
            if (!blockedWebsites || blockedWebsites.length === 0) {
                tbody.appendChild(createEmptyRow("No websites blocked yet."));
            }
            else{
                const allowedWebsitesForDeletions = new Set(Array.from(blockData.allowedForUnblockWebsites ?? []));
                Array.from(blockedWebsites).forEach((item, index) => {
                    if(setOfEntries.has(item)){
                        return;
                    }
                    const isAllowedToDelete = allowedWebsitesForDeletions.has(item);
                    const row = createDataRow(item, index, isAllowedToDelete);
                    tbody.appendChild(row);
                    setOfEntries.add(item);
                });
                list = Array.from(blockedWebsites);
            }
    } catch(error){
        console.log(error);
        tbody.appendChild(createEmptyRow("No websites blocked yet."));
    }
}

function getButtonTextAndColour(isAllowedToDelete){
    if(isAllowedToDelete){
        return {
            label: "Delete",
            color: '#fff'
        }
    }

    return {
        color: 'green',
        label: "Prepare for Deletion"
    }
}

function closeModal(modal){
    modal.style.display = 'none';
}

function isValidHostname(host) {
    if (!host || typeof host !== 'string') return false;
    if (host.length > 253) return false;
    if (/[^\x20-\x7E]/.test(host)) return false;              
    if (/[ \t\r\n]/.test(host)) return false;
    if (host.endsWith('.')) host = host.slice(0, -1);
    const ipLike = /^\d{1,3}(\.\d{1,3}){3}$/;
    if (ipLike.test(host)) return false;
    const label = /^(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)$/i;
    const parts = host.split('.');
    if (parts.length < 2) return false;
    return parts.every(p => label.test(p)) &&
            (/^[a-z]{2,63}$/i.test(parts[parts.length - 1]) ||
            /^xn--[a-z0-9-]{2,59}$/i.test(parts[parts.length - 1]));
}

function normalizeHost(input) {
    let s = String(input || '').trim();
    if (!s) return null;
    if (/[\r\n]/.test(s)) return null;
    if (!/^https?:\/\//i.test(s)) s = 'http://' + s;
    try {
        const url = new URL(s);
        let h = url.hostname.toLowerCase();
        h = h.replace(/^www\./, '');
        if (!isValidHostname(h)) return null;
        return h;
    } catch {
        return null;
    }
}

async function addToBlockListsForWebsites(modal, domain) {
    const host = normalizeHost(domain);
    if (!host) {
        alert("Please enter a valid website domain (e.g., example.com).");
        return false;
    }

    return invoke('add_block_website', { site: host })
        .then(() => {
            if (modal) {
                closeModal(modal);
                window.location.reload();
            }
            return true;
        })
        .catch((error) => {
            console.error(error);
            alert("Unable to block the given website.");
            return false;
        });
}

function listenForPrimeDeletion(){
    listen('show_delay_for_prime_deletion', (event) => {
        const payload = event?.payload || {};
        const settingId = payload.settingId;

        if (settingId) {
            openConfirmationDialog(settingId);
        }
    });
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
