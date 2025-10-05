const { invoke } = window.__TAURI__.tauri;
const { listen } = window.__TAURI__.event;
const HOSTS_FILE = 'C:\\Windows\\System32\\drivers\\etc\\hosts';

let list;

function init(){
    if(!window){
        return;
    }

    window.addEventListener('DOMContentLoaded', () => {
        const modal = document.getElementById('modal');
        const input = document.getElementById('modal-input');
        const saveBtn = document.getElementById('save-btn');
        const cancelBtn = document.getElementById('cancel-btn');
        const addBtn = document.getElementById('add-btn');

        void renderTable();

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
                        void renderTable();
                        modal.style.display = 'none';
                    }
                })
                .catch(error => {
                    console.error('Error blocking website:', error);
                    alert(`Failed to block website: ${error.message}`);
                });
        });
    });

    window.removeItem = (index) => removeWebsite(index);

    initializeListener();
}

function removeWebsite(index){
    if(!list || index < 0 || index > list.length){
        return;
    }
    const site = list[index];
    invoke('remove_block_website', {site})
        .then(() => renderTable())
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
            const payload = {
                itemType: "website",
                name: item
            }
            invoke("prime_for_deletion", payload);
        }
    };

    return button;
}

async function renderTable() {
    const tbody = document.querySelector('#data-table tbody');
    tbody.innerHTML = '';

    return invoke('get_block_data')
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

async function addToBlockListsForWebsites(modal, domain) {
    const normalizeDomain = (input) => {
        try {
            if (!input.startsWith('http')) input = 'http://' + input;
            const url = new URL(input);
            let hostname = url.hostname.toLowerCase();
            return hostname.replace(/^www\./, '');
        } catch {
            return null;
        }
    }

    if (normalizeDomain(domain) === null) {
        alert("Invalid website address. Please enter a valid URL.");
        return false;
    }

    return invoke('add_block_website', {site: domain})
        .then(() => {
            if(modal != null){
                closeModal(modal);
                void renderTable();
            }
        })
        .catch((error) => {
            console.log(error);
            alert("Unable to block given website");
        });
}