const { invoke } = window.__TAURI__.tauri;
const { listen } = window.__TAURI__.event;
const { WebviewWindow } = window.__TAURI__.window;

const filename = 'blockData.json';
let confirmDialog;

function getBlockedAppsList() {
    return getBlockData()
        .then(blockData => {
            const blockedApps = blockData?.blockedApps || [];
            const allowedForDelete = blockData?.allowedForUnblockApps || [];
            return Array.from(blockedApps)
                .map(blockedApp => {
                    const processName = blockedApp.processName;
                    return {
                        ...blockedApp,
                        isAllowedForDelete: allowedForDelete.includes(processName)
                    }
                });
        })
        .catch(error => {
            console.error('Failed to get blocked apps:', error);
            return [];
        });
}

function getBlockData() {
    return invoke('get_block_data')
        .then(data => data || { blockedApps: [] })
        .catch(error => {
            console.error('Failed to get block data:', error);
            return { blockedApps: [] };
        });
}

function getAllInstalledApps() {
    return invoke('get_all_installed_apps')
        .then(apps => {
            console.log("apps:", apps);
            return apps || [];
        })
        .catch(error => {
            console.error('Failed to get installed apps:', error);
            return [];
        });
}


function createEmptyRow(message) {
  const row = document.createElement('tr');
  const cell = document.createElement('td');
  cell.colSpan = 2;
  cell.textContent = message;
  row.appendChild(cell);
  return row;
}

function processBlockedAppsAndRenderTable(list) {
    const tbody = document.querySelector('#data-table tbody');
    if (!tbody) return;

    if (tbody.replaceChildren) tbody.replaceChildren();
    else tbody.innerHTML = '';

    if (!list || list.length === 0) {
        tbody.appendChild(createEmptyRow('No apps blocked yet.'));
        return;
    }

     const uniqueList = Array.from(
        new Map(
            (list || []).map(item => [item?.processName?.toLowerCase()?.trim(), item])
        ).values()
    );

    if (uniqueList.length === 0) {
        tbody.appendChild(createEmptyRow('No apps blocked yet.'));
        return;
    }

    uniqueList.forEach(async (item, index) => {
        const row = document.createElement('tr');

        const nameCell = document.createElement('td');
        nameCell.textContent = item.displayName || 'Unknown App';

        const deleteCell = document.createElement('td');
        const deleteButton = createDeleteButton(item, index, item?.isAllowedForDelete);
        deleteCell.appendChild(deleteButton);

        const iconCell = document.createElement('td');
        const icon = document.createElement('span');
        icon.textContent = '✅';

        if(!item?.isAllowedForDelete){
            const key = "allowedForUnblockApps-->" + item.processName;
            const changeStatus = await invoke("get_change_status", {settingId: key});
            if(changeStatus.isChanging){
                icon.textContent = '⏱️';
            }
        }

        iconCell.appendChild(icon);
        row.appendChild(iconCell);

        row.appendChild(nameCell);
        row.appendChild(deleteCell);

        tbody.appendChild(row);
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
            if(!item || !item.processName || !item.processName.trim()){
                return false;
            }

            const key = "allowedForUnblockApps-->" + item.processName;
            invoke("get_change_status", {settingId: key})
                .then(statusChange => {
                    if(statusChange.isChanging){
                        openConfirmationDialog(key);
                    }
                    else{
                        const payload = {
                            itemType: "app",
                            name: item.processName
                        }
                        invoke("prime_for_deletion", payload)
                            .then(() => window.location.reload());
                    }
                });
        }
    };

    return button;
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


function initializeListener(){
    listen("block-data-updated").then(() => renderTable());
}

function renderTable(list = null) {
    if (list) {
        return Promise.resolve(processBlockedAppsAndRenderTable(list));
    }

    return getBlockedAppsList()
        .then(blockedApps => {
            processBlockedAppsAndRenderTable(blockedApps);
        })
        .catch(error => {
            console.error('Failed to render table:', error);
            processBlockedAppsAndRenderTable([]);
        });
}

function removeItemAtIndex(index) {
    return getBlockedAppsList()
        .then(list => {
            if (!list || list.length === 0) {
                return;
            }

            const updatedList = [...list];
            updatedList.splice(index, 1);

            return saveBlockedAppsList(updatedList)
                .then(() => {
                    console.log("Deletion of object at index: " + index + " was successful");
                    processBlockedAppsAndRenderTable(updatedList);
                });
        })
        .catch(error => {
            console.error('Failed to remove item:', error);
        });
}

function saveBlockedAppsList(newList) {
    return getBlockData()
        .then(blockData => {
            const newData = {
                ...blockData,
                blockedApps: newList
            };
            return invoke('save_block_data', { data: newData })
                .then(() => newData)
                .catch(err => {
                    console.error('Failed to save blocked apps list:', err);
                    throw err;
                });
        })
        .catch(error => {
            console.error('Failed to save blocked apps list:', error);
            throw error;
        });
}

function renderAppSearchModal() {
    const modal = document.getElementById('appSelectionModal');
    const tbody = document.getElementById('appList');
    const searchInput = document.getElementById('appSearchInput');
    const cancelBtn = document.getElementById('cancelSelectionBtn');
    const blockBtn = document.getElementById('blockSelectedBtn');
    const topCancelButton = document.getElementById("closeAppModal");

    tbody.innerHTML = '<tr><td colspan="2" style="text-align: center; padding: 20px;">Loading apps...</td></tr>';
    modal.style.display = 'block';

    Promise.all([
        getAllInstalledApps(),
        getBlockedAppsList()
    ])
        .then(([installedApps, blockedApps]) => {
            const blockedProcessNames = new Set((blockedApps || []).map(app => app.processName));
            const availableApps = (installedApps || []).filter(app =>
                app.processName && !blockedProcessNames.has(app.processName)
            );

            const renderList = (filteredApps) => {
                tbody.innerHTML = '';

                if (filteredApps.length === 0) {
                    tbody.innerHTML = '<tr><td colspan="2" style="text-align: center; padding: 20px;">No apps found</td></tr>';
                    return;
                }

                filteredApps.forEach((app) => {
                    const row = document.createElement('tr');

                    const selectCell = document.createElement('td');
                    selectCell.style.padding = '10px';
                    const checkbox = document.createElement('input');
                    checkbox.type = 'checkbox';
                    checkbox.dataset.processName = app.processName;
                    checkbox.dataset.displayName = app.displayName;
                    selectCell.appendChild(checkbox);

                    const nameCell = document.createElement('td');
                    nameCell.textContent = app.displayName || 'Unknown App';
                    nameCell.style.padding = '10px';
                    nameCell.style.color = 'black';

                    row.appendChild(selectCell);
                    row.appendChild(nameCell);
                    tbody.appendChild(row);
                });
            };

            searchInput.value = '';
            renderList(availableApps);

            searchInput.oninput = () => {
                const keyword = searchInput.value.toLowerCase();
                const filtered = availableApps.filter(app =>
                    app.displayName && app.displayName.toLowerCase().includes(keyword)
                );
                renderList(filtered);
            };

            blockBtn.onclick = () => {
                const checkboxes = tbody.querySelectorAll('input[type="checkbox"]:checked');

                return getBlockedAppsList()
                    .then(currentBlockedApps => {
                        const updatedList = [...(currentBlockedApps || [])];

                        checkboxes.forEach(cb => {
                            const processName = cb?.dataset?.processName;
                            const displayName = cb?.dataset?.displayName;

                            if (processName && !updatedList.some(app => app.processName === processName)) {
                                updatedList.push({ processName, displayName });
                            }
                        });

                        return saveBlockedAppsList(updatedList);
                    })
                    .then(() => renderTable())
                    .then(() => modal.style.display = 'none')
                    .catch(error => {
                        console.error('Failed to block selected apps:', error);
                    });
            };
        })
        .catch(error => {
            console.error('Failed to load apps:', error);
            tbody.innerHTML = '<tr><td colspan="2" style="text-align: center; padding: 20px; color: red;">Failed to load apps</td></tr>';
        });

    cancelBtn.onclick = () => modal.style.display = 'none';
    topCancelButton.onclick = () => modal.style.display = 'none';
}

window.removeItem = function(index) {
    removeItemAtIndex(index);
};

window.addEventListener('DOMContentLoaded', () => {
    const modal = document.getElementById('modal');
    const input = document.getElementById('modal-input');
    const saveBtn = document.getElementById('save-btn');
    const cancelBtn = document.getElementById('cancel-btn');
    const addBtn = document.getElementById('add-btn');
    const title = document.getElementById('title');

    if (title) title.innerText = 'Blocked Apps';

    initializeListener();

    Promise.all([
        getAllInstalledApps(),
        renderTable()
    ])
        .then(() => {
            console.log('App blocking interface loaded successfully');
        })
        .catch(error => {
            console.error('Failed to initialize app blocking interface:', error);
        });

    if (addBtn) {
        addBtn.addEventListener('click', renderAppSearchModal);
    }

    if (cancelBtn) {
        cancelBtn.addEventListener('click', () => {
            if (modal) modal.style.display = 'none';
        });
    }

    if (saveBtn) {
        saveBtn.addEventListener('click', () => {
            const value = input?.value?.trim();

            if (!value) return;

            getBlockData()
                .then(blockData => {
                    const list = [...(blockData.blockedApps || [])];
                    list.push(value);

                    const newData = { ...blockData, blockedApps: list };
                    invoke('save_block_data', { data: newData });

                    return renderTable();
                })
                .then(() => {
                    if (modal) modal.style.display = 'none';
                    if (input) input.value = '';
                })
                .catch(error => {
                    console.error('Failed to save new blocked app:', error);
                });
        });
    }
});