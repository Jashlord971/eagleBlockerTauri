const { invoke } = window.__TAURI__.tauri;
const { listen } = window.__TAURI__.event;
const { WebviewWindow } = window.__TAURI__.window;
const HOSTS_FILE = 'C:\\Windows\\System32\\drivers\\etc\\hosts';

let confirmDialog;

let list;

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
                    }
                })
                .catch(error => {
                    console.error('Error blocking website:', error);
                    alert(`Failed to block website: ${error.message}`);
                });
        });

        void renderTable();
    });

    window.removeItem = (index) => removeWebsite(index);
}

function removeWebsite(index){
    if(!list || index < 0 || index > list.length){
        return;
    }
    const site = list[index];
    invoke('remove_block_website', {site})
        .then(() => window.location.reload())
        .catch(error => console.log(error));
}

function initializeListener(){
    listen("block-data-updated", () => renderTable());
}

init();

function createEmptyRow(message) {
    const row = document.createElement('tr');
    const cell = document.createElement('td');
    cell.colSpan = 3;
    cell.textContent = message;
    row.appendChild(cell);
    return row;
}

function createDataRow(item, index, isAllowedToDelete) {
    const row = document.createElement('tr');
    const iconCell = document.createElement('td');
    const icon = document.createElement('span');
    icon.textContent = '✅';
    icon.setAttribute('data-timr', '1');

    if (!isAllowedToDelete) {
        const key = "allowedForUnblockWebsites-->" + item;
        invoke("get_change_status", { settingId: key })
            .then(statusChange => {
                if (statusChange && statusChange.isChanging) {
                    icon.textContent = '⏱️';
                }
            });
    }
    iconCell.appendChild(icon);

    const nameCell = document.createElement('td');
    nameCell.textContent = typeof item === 'string' ? item : item.displayName;

    const deleteCell = document.createElement('td');
    const deleteButton = createDeleteButton(item, index, isAllowedToDelete);
    deleteCell.appendChild(deleteButton);

    row.appendChild(iconCell);
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
                        invoke("prime_for_deletion", payload).then(() => {
                            const row = button.closest('tr');
                            const timr = row?.querySelector('[data-timr]');
                            if (timr) timr.textContent = '⏱️';
                        });
                    }
                });
        }
    };

    return button;
}

async function renderTable() {
    const tbody = document.querySelector('#data-table tbody');
    tbody.innerHTML = '';
    
    return invoke('get_block_data_for_block_websites')
        .then(blockData => {
            const blockedWebsites = new Set(Array.from(blockData?.blockedWebsites || []));
            if (!blockedWebsites || blockedWebsites.length === 0) {
                tbody.appendChild(createEmptyRow("No websites blocked yet."));
            }
            else{
                const allowedWebsitesForDeletions = new Set(Array.from(blockData.allowedForUnblockWebsites ?? []));
                Array.from(blockedWebsites).forEach((item, index) => {
                    const isAllowedToDelete = allowedWebsitesForDeletions.has(item);
                    const row = createDataRow(item, index, isAllowedToDelete);
                    tbody.appendChild(row);
                });
                list = Array.from(blockedWebsites);
            }
        });
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
