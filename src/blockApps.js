const { invoke } = window.__TAURI__.tauri;
const filename = 'blockData.json';

let cachedData = {
    blockedApps: null,
    installedApps: null,
    blockData: null
};

let pendingRequests = {
    blockedApps: null,
    installedApps: null,
    blockData: null
};

const CACHE_TTL = 5 * 60 * 1000;
let cacheTimestamps = {
    installedApps: 0,
    blockData: 0
};

function getBlockedAppsList() {
    if (cachedData.blockedApps) {
        return Promise.resolve(cachedData.blockedApps);
    }

    if (pendingRequests.blockedApps) {
        return pendingRequests.blockedApps;
    }

    pendingRequests.blockedApps = getBlockData()
        .then(blockData => {
            const apps = blockData.blockedApps || [];
            cachedData.blockedApps = apps;
            pendingRequests.blockedApps = null;
            return apps;
        })
        .catch(error => {
            console.error('Failed to get blocked apps:', error);
            pendingRequests.blockedApps = null;
            return [];
        });

    return pendingRequests.blockedApps;
}

function getBlockData() {
    const now = Date.now();
    const isExpired = now - cacheTimestamps.blockData > CACHE_TTL;

    if (cachedData.blockData && !isExpired) {
        return Promise.resolve(cachedData.blockData);
    }

    if (pendingRequests.blockData) {
        return pendingRequests.blockData;
    }

    pendingRequests.blockData = invoke('get_block_data')
        .then(data => {
            cachedData.blockData = data || { blockedApps: [] };
            cacheTimestamps.blockData = now;
            pendingRequests.blockData = null;
            return cachedData.blockData;
        })
        .catch(error => {
            console.error('Failed to get block data:', error);
            pendingRequests.blockData = null;
            return { blockedApps: [] };
        });

    return pendingRequests.blockData;
}

function getAllInstalledApps() {
    const now = Date.now();
    const isExpired = now - cacheTimestamps.installedApps > CACHE_TTL;

    if (cachedData.installedApps && !isExpired) {
        return Promise.resolve(cachedData.installedApps);
    }

    if (pendingRequests.installedApps) {
        return pendingRequests.installedApps;
    }

    pendingRequests.installedApps = invoke('get_all_installed_apps')
        .then(apps => {
            cachedData.installedApps = apps || [];
            cacheTimestamps.installedApps = now;
            pendingRequests.installedApps = null;
            return cachedData.installedApps;
        })
        .catch(error => {
            console.error('Failed to get installed apps:', error);
            pendingRequests.installedApps = null;
            return [];
        });

    return pendingRequests.installedApps;
}

function processBlockedAppsAndRenderTable(list) {
    const tbody = document.querySelector('#data-table tbody');
    if (!tbody) return;

    tbody.innerHTML = '';

    list.forEach((item, index) => {
        const row = document.createElement('tr');

        const nameCell = document.createElement('td');
        nameCell.textContent = item.displayName || 'Unknown App';

        const deleteCell = document.createElement('td');
        const deleteButton = getDeleteButton();
        deleteButton.onclick = () => removeItem(index);
        deleteCell.appendChild(deleteButton);

        row.appendChild(nameCell);
        row.appendChild(deleteCell);
        tbody.appendChild(row);
    });
}

function renderTable(list = null) {
    if (list) {
        return processBlockedAppsAndRenderTable(list);
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

function getDeleteButton(text = 'Delete') {
    const button = document.createElement('button');
    button.textContent = text;
    button.style.backgroundColor = '#FF5555';
    button.style.color = '#fff';
    button.style.border = 'none';
    button.style.padding = '5px 10px';
    button.style.borderRadius = '5px';
    button.style.cursor = 'pointer';
    return button;
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
                    cachedData.blockedApps = updatedList;
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

            invoke('save_block_data', {data: newData});

            cachedData.blockData = newData;
            cachedData.blockedApps = newList;
            cacheTimestamps.blockData = Date.now();
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

    // Show loading state
    tbody.innerHTML = '<tr><td colspan="2" style="text-align: center; padding: 20px;">Loading apps...</td></tr>';
    modal.style.display = 'block';

    Promise.all([
        getAllInstalledApps(),
        getBlockedAppsList()
    ])
        .then(([installedApps, blockedApps]) => {
            const blockedProcessNames = new Set(blockedApps.map(app => app.processName));
            const availableApps = installedApps.filter(app =>
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
                        const updatedList = [...currentBlockedApps];

                        checkboxes.forEach(cb => {
                            const processName = cb?.dataset?.processName;
                            const displayName = cb?.dataset?.displayName;

                            if (processName && !updatedList.some(app => app.processName === processName)) {
                                updatedList.push({ processName, displayName });
                            }
                        });

                        return saveBlockedAppsList(updatedList);
                    })
                    .then(() => {
                        return renderTable();
                    })
                    .then(() => {
                        modal.style.display = 'none';
                    })
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

    // Pre-load data
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

                    cachedData.blockData = newData;
                    cachedData.blockedApps = list;

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