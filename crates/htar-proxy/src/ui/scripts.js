// Modular Frontend Controller for HTAR Enterprise Gateway Dashboard
async function fetchAPI(endpoint, options = {}) {
    try {
        options.credentials = 'same-origin';
        options.headers = options.headers || {};
        const savedToken = localStorage.getItem('htar_token');
        if (savedToken) {
            options.headers['Authorization'] = `Bearer ${savedToken}`;
        }
        const response = await fetch(endpoint, options);
        if (!response.ok) return null;
        return await response.json();
    } catch (err) {
        console.error(`API Fetch Error [${endpoint}]:`, err);
        return null;
    }
}

function setText(id, text) {
    const el = document.getElementById(id);
    if (el) el.innerText = text;
}

function switchView(viewName) {
    document.querySelectorAll('.console-view-panel').forEach(el => el.classList.add('hidden'));
    document.querySelectorAll('.nav-sidebar-btn').forEach(el => {
        el.classList.remove('bg-zinc-950', 'text-emerald-400', 'border-emerald-500/30', 'font-bold', 'shadow-sm');
        el.classList.add('border-transparent', 'text-zinc-400');
    });

    const activeView = document.getElementById(`view-${viewName}`);
    if (activeView) activeView.classList.remove('hidden');

    const activeBtn = document.getElementById(`nav-btn-${viewName}`);
    if (activeBtn) {
        activeBtn.classList.add('bg-zinc-950', 'text-emerald-400', 'border-emerald-500/30', 'font-bold', 'shadow-sm');
        activeBtn.classList.remove('border-transparent', 'text-zinc-400');
    }
}

async function syncVault() {
    const btn = document.getElementById('vault-btn');
    if (btn) btn.innerHTML = '<i class="fa-solid fa-spinner animate-spin"></i>';
    
    const res = await fetchAPI('/admin/v1/vault/sync', { method: 'POST' });
    if (res && res.status === "synced_from_vault") {
        alert(`HashiCorp Vault Secrets Synced Successfully!\nRetrieved ${res.retrieved_keys_count} secrets`);
    } else {
        alert('Vault Sync Triggered (Sidecar /vault/secrets/env active)');
    }
    if (btn) btn.innerHTML = '<i class="fa-solid fa-vault"></i> Sync Vault';
    refreshData();
}

async function toggleAuthEnforcement() {
    await fetchAPI('/admin/v1/auth/toggle', { method: 'POST' });
    refreshData();
}

async function logout() {
    try {
        await fetchAPI('/admin/v1/auth/logout', { method: 'POST' });
    } catch (e) {
        console.error("Logout API error:", e);
    }
    localStorage.removeItem('htar_token');
    document.cookie = "htar_session_token=; Path=/; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT";
    showLoginScreen();
}

function showLoginScreen() {
    const loginEl = document.getElementById('login-container');
    const headerEl = document.getElementById('header-navbar');
    const consoleEl = document.getElementById('console-layout');
    if (loginEl) loginEl.classList.remove('hidden');
    if (headerEl) headerEl.classList.add('hidden');
    if (consoleEl) consoleEl.classList.add('hidden');
}

function showDashboard() {
    const loginEl = document.getElementById('login-container');
    const headerEl = document.getElementById('header-navbar');
    const consoleEl = document.getElementById('console-layout');
    if (loginEl) loginEl.classList.add('hidden');
    if (headerEl) headerEl.classList.remove('hidden');
    if (consoleEl) consoleEl.classList.remove('hidden');
}

function copyK8sToken() {
    const tokenBox = document.getElementById('full-k8s-token-box');
    if (tokenBox) {
        navigator.clipboard.writeText(tokenBox.innerText.trim());
        alert('K8s ServiceAccount Bearer Token copied to clipboard!');
    }
}

async function performLogin(username, password) {
    const errDiv = document.getElementById('standalone-err');
    if (errDiv) errDiv.classList.add('hidden');

    try {
        const res = await fetch('/admin/v1/auth/login', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ username, password })
        });
        const data = await res.json();
        if (res.ok && data.status === "authenticated" && data.token) {
            localStorage.setItem('htar_token', data.token);
            showDashboard();
            await refreshData();
        } else {
            if (errDiv) {
                errDiv.classList.remove('hidden');
                errDiv.innerText = data.error || "Invalid username or password";
            }
        }
    } catch (err) {
        if (errDiv) {
            errDiv.classList.remove('hidden');
            errDiv.innerText = "Network error connecting to Auth API";
        }
    }
}

async function toggleRouteAuth(routeId) {
    await fetchAPI('/admin/v1/routes/toggle-auth', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ route_id: routeId })
    });
    refreshData();
}

async function toggleSwitchboard(routeId) {
    await fetchAPI('/admin/v1/switchboard/toggle', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ route_id: routeId })
    });
    refreshData();
}

async function deleteUser(username) {
    if (!confirm(`Are you sure you want to delete identity user '${username}' from MySQL storage?`)) return;
    const res = await fetchAPI('/admin/v1/users/delete', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ username })
    });
    if (res && res.status === "deleted") {
        refreshData();
    } else {
        alert(res?.error || "Failed to delete user identity");
    }
}

async function refreshData() {
    const icon = document.getElementById('refresh-icon');
    if (icon) icon.classList.add('animate-spin');

    try {
        const authStatus = await fetchAPI('/admin/v1/auth/status');
        const toggleStatusEl = document.getElementById('auth-toggle-status');
        
        if (authStatus) {
            if (toggleStatusEl) {
                if (authStatus.auth_enabled) {
                    toggleStatusEl.innerText = "ENFORCED";
                    toggleStatusEl.className = "px-2 py-0.5 rounded text-[10px] font-bold bg-emerald-500/10 text-emerald-400 border border-emerald-500/20";
                } else {
                    toggleStatusEl.innerText = "BYPASSED";
                    toggleStatusEl.className = "px-2 py-0.5 rounded text-[10px] font-bold bg-amber-500/10 text-amber-400 border border-amber-500/20";
                }
            }

            if (authStatus.user) {
                setText('user-display', authStatus.user.username);
                setText('role-badge', authStatus.user.role);
            }

            if (authStatus.authenticated) {
                showDashboard();
            } else if (authStatus.auth_enabled && !authStatus.authenticated) {
                if (!localStorage.getItem('htar_token')) {
                    showLoginScreen();
                }
            }
        }

        const roiData = await fetchAPI('/admin/v1/roi');
        if (roiData) {
            if (roiData.total_requests !== undefined) {
                setText('metric-total-requests', roiData.total_requests.toLocaleString());
            }
            if (roiData.req_per_sec !== undefined) {
                setText('metric-req-rate', `${roiData.req_per_sec} req/s`);
                setText('header-req-rate', `${roiData.req_per_sec} req/s`);
            }
            if (roiData.uptime_seconds !== undefined) {
                const s = roiData.uptime_seconds;
                const hrs = Math.floor(s / 3600);
                const mins = Math.floor((s % 3600) / 60);
                const secs = s % 60;
                setText('metric-uptime', hrs > 0 ? `${hrs}h ${mins}m` : `${mins}m ${secs}s`);
            }
            if (roiData.p50_latency_us) {
                setText('metric-p50-latency', `${roiData.p50_latency_us} µs`);
            }
            if (roiData.p99_latency_us) {
                setText('metric-p99-latency', `${roiData.p99_latency_us} µs`);
            }
            if (roiData.avg_pod_memory_mb) {
                setText('metric-memory', `${roiData.avg_pod_memory_mb} MB`);
            }
            if (roiData.cache_hit_ratio !== undefined) {
                setText('metric-cache-ratio', `${roiData.cache_hit_ratio}%`);
            }
            if (roiData.cache_hits !== undefined) {
                setText('metric-cache-hits', roiData.cache_hits.toLocaleString());
            }
            if (roiData.cache_misses !== undefined) {
                setText('metric-cache-misses', roiData.cache_misses.toLocaleString());
            }
        }

        const usersData = await fetchAPI('/admin/v1/users');
        if (usersData && usersData.users) renderUsers(usersData.users);

        const endpointsData = await fetchAPI('/admin/v1/endpoints');
        if (endpointsData && endpointsData.endpoints) {
            renderEndpoints(endpointsData.endpoints);
            renderDomainCards(endpointsData.endpoints);
            setText('domain-count-badge', `${endpointsData.endpoints.length} Discovered Services`);
        }

        const switchboardData = await fetchAPI('/admin/v1/switchboard');
        if (switchboardData && switchboardData.entries) renderSwitchboard(switchboardData.entries);

        const wasmData = await fetchAPI('/admin/v1/plugins/wasm');
        if (wasmData && wasmData.wasm_plugins) renderWasmPlugins(wasmData.wasm_plugins);

    } catch (e) {
        console.error("Refresh Error:", e);
    } finally {
        if (icon) setTimeout(() => icon.classList.remove('animate-spin'), 400);
    }
}

function renderDomainCards(endpoints) {
    const container = document.getElementById('domain-cards-container');
    if (!container) return;
    if (!endpoints || endpoints.length === 0) {
        container.innerHTML = '<div class="text-xs text-zinc-500 font-mono">No active endpoints discovered.</div>';
        return;
    }

    container.innerHTML = endpoints.map(e => `
        <div class="bg-zinc-950 border border-zinc-800 rounded-lg p-4 flex justify-between items-center shadow-sm">
            <div class="flex items-center gap-3">
                <div class="w-2 h-7 ${e.healthy ? 'bg-emerald-500' : 'bg-rose-500'} rounded-full"></div>
                <div>
                    <h4 class="font-bold text-xs text-zinc-100 font-mono">${e.route_id}</h4>
                    <p class="text-[11px] font-mono text-zinc-400 mt-0.5">Path: <span class="text-zinc-200">${(e.paths || []).join(', ')}</span> | P50: <span class="text-zinc-200">240µs</span></p>
                </div>
            </div>
            <span class="text-xs font-bold ${e.healthy ? 'text-emerald-400' : 'text-rose-400'} font-mono">${e.healthy ? 'NOMINAL' : 'UNHEALTHY'}</span>
        </div>
    `).join('');
}

function renderUsers(list) {
    const tbody = document.getElementById('users-table-body');
    if (!tbody) return;
    tbody.innerHTML = list.map(u => `
        <tr class="hover:bg-zinc-800/40 text-xs font-mono">
            <td class="px-6 py-4 font-bold text-emerald-400">${u.username}</td>
            <td class="px-6 py-4">
                <span class="px-2 py-0.5 rounded text-[10px] font-bold ${u.role === 'SuperAdmin' ? 'bg-purple-500/10 text-purple-300 border border-purple-500/20' : (u.role === 'Operator' ? 'bg-emerald-500/10 text-emerald-400 border border-emerald-500/20' : 'bg-zinc-800 text-zinc-300')}">
                    ${u.role}
                </span>
            </td>
            <td class="px-6 py-4 text-zinc-300">${(u.permissions || []).join(', ')}</td>
            <td class="px-6 py-4 text-amber-400 flex items-center gap-1.5">
                <i class="fa-solid fa-database text-[10px]"></i> ${u.mysql_storage_status || 'MySQL (Synced)'}
            </td>
            <td class="px-6 py-4 text-right">
                ${u.username === 'admin' ? '<span class="text-zinc-600 text-[10px]">Root System Identity</span>' : `
                    <button onclick="deleteUser('${u.username}')" class="px-2.5 py-1 rounded text-xs font-bold text-rose-400 bg-rose-500/10 border border-rose-500/20 hover:bg-rose-500/20 transition">
                        <i class="fa-solid fa-trash-can"></i> Delete
                    </button>
                `}
            </td>
        </tr>
    `).join('');
}

function renderEndpoints(list) {
    const tbody = document.getElementById('endpoints-table-body');
    if (!tbody) return;
    tbody.innerHTML = list.map(e => `
        <tr class="hover:bg-zinc-800/40 text-xs font-mono">
            <td class="px-6 py-4 text-emerald-400 font-bold">${e.route_id}</td>
            <td class="px-6 py-4 text-zinc-100">${(e.paths || []).join(', ')}</td>
            <td class="px-6 py-4 text-zinc-400">${(e.hosts || []).join(', ')}</td>
            <td class="px-6 py-4 text-zinc-300">${(e.upstreams || []).map(u => u.url).join('<br>')}</td>
            <td class="px-6 py-4">
                <span class="inline-flex items-center px-2 py-0.5 rounded text-[10px] font-bold ${e.healthy ? 'bg-emerald-500/10 text-emerald-400 border border-emerald-500/20' : 'bg-rose-500/10 text-rose-400 border border-rose-500/20'}">
                    ${e.healthy ? 'HEALTHY' : 'UNHEALTHY'}
                </span>
            </td>
            <td class="px-6 py-4 text-zinc-400">${e.cache_enabled ? 'Enabled (' + (e.cache_ttl_secs || 60) + 's)' : 'Disabled'}</td>
        </tr>
    `).join('');
}

function renderSwitchboard(entries) {
    const tbody = document.getElementById('switchboard-table-body');
    if (!tbody) return;
    tbody.innerHTML = entries.map(s => `
        <tr class="hover:bg-zinc-800/40 text-xs font-mono">
            <td class="px-6 py-4 text-emerald-400 font-bold">${s.route_id}<br><span class="text-zinc-500 font-normal text-[11px]">${s.service_name}</span></td>
            <td class="px-6 py-4 text-zinc-100">${(s.paths || []).join(', ')}</td>
            <td class="px-6 py-4">
                <span class="px-2 py-0.5 rounded text-[10px] font-bold bg-emerald-500/10 text-emerald-400 border border-emerald-500/20">
                    ${s.gateway_api_status}
                </span>
            </td>
            <td class="px-6 py-4">
                <span class="${s.auth_enabled ? 'text-emerald-400 font-bold' : 'text-zinc-400'}">
                    ${s.auth_enabled ? '🔐 Auth Protected' : '🌐 Public Endpoint'}
                </span>
            </td>
            <td class="px-6 py-4 text-right space-x-2">
                <button onclick="toggleRouteAuth('${s.route_id}')" class="px-2.5 py-1 rounded text-xs font-bold transition ${s.auth_enabled ? 'bg-purple-500/10 text-purple-300 border border-purple-500/20 hover:bg-purple-500/20' : 'bg-zinc-800 text-zinc-300 border border-zinc-700 hover:bg-zinc-700'}">
                    ${s.auth_enabled ? 'Disable Auth' : 'Enable Auth Portal'}
                </button>
                <button onclick="toggleSwitchboard('${s.route_id}')" class="px-2.5 py-1 rounded text-xs font-bold transition ${s.cache_enabled ? 'bg-amber-500/10 text-amber-300 border border-amber-500/20 hover:bg-amber-500/20' : 'bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 hover:bg-emerald-500/20'}">
                    ${s.cache_enabled ? 'Disable Cache' : 'Enable O(1) Cache'}
                </button>
            </td>
        </tr>
    `).join('');
}

function renderWasmPlugins(list) {
    const tbody = document.getElementById('wasm-plugins-table-body');
    if (!tbody) return;
    const defaultPlugins = ['auth_header_plugin', 'sample_plugin'];
    const allPlugins = Array.from(new Set([...(list || []), ...defaultPlugins]));
    tbody.innerHTML = allPlugins.map(name => `
        <tr class="hover:bg-zinc-800/40 text-xs font-mono">
            <td class="px-4 py-3 font-bold text-purple-300">${name}.wasm</td>
            <td class="px-4 py-3 text-zinc-400">Wasmtime v21 (Cranelift JIT)</td>
            <td class="px-4 py-3 text-zinc-300">on_request_headers</td>
            <td class="px-4 py-3 text-emerald-400 font-bold">~45µs</td>
            <td class="px-4 py-3">
                <span class="px-2 py-0.5 rounded text-[10px] font-bold bg-emerald-500/10 text-emerald-400 border border-emerald-500/20">
                    COMPILED & ACTIVE
                </span>
            </td>
        </tr>
    `).join('');
}

function toggleModal(id) {
    const modal = document.getElementById(id);
    if (modal) modal.classList.toggle('hidden');
}

// Event Listeners for Forms using fetchAPI
document.addEventListener('DOMContentLoaded', () => {
    const wasmForm = document.getElementById('form-wasm-upload');
    if (wasmForm) {
        wasmForm.addEventListener('submit', async (e) => {
            e.preventDefault();
            const pluginName = document.getElementById('wasm-plugin-name').value;
            const fileInput = document.getElementById('wasm-file-input');
            
            if (!fileInput.files || !fileInput.files[0]) {
                alert('Please select a .wasm plugin binary file to compile.');
                return;
            }

            const bodyBytes = await fileInput.files[0].arrayBuffer();
            const savedToken = localStorage.getItem('htar_token');
            const headers = { 'X-Plugin-Name': pluginName };
            if (savedToken) headers['Authorization'] = `Bearer ${savedToken}`;

            const res = await fetch('/admin/v1/plugins/wasm', {
                method: 'POST',
                headers: headers,
                body: bodyBytes
            });

            if (res.ok) {
                alert(`Wasm Plugin '${pluginName}' compiled & registered successfully in Wasmtime Engine!`);
                refreshData();
            } else {
                const err = await res.text();
                alert(`Wasm compilation error: ${err}`);
            }
        });
    }

    const routeForm = document.getElementById('form-route');
    if (routeForm) {
        routeForm.addEventListener('submit', async (e) => {
            e.preventDefault();
            const svcId = document.getElementById('route-svc-id').value;
            const pathsRaw = document.getElementById('route-paths').value;
            const stripPath = document.getElementById('route-strip-path').checked;
            const enableCache = document.getElementById('route-enable-cache').checked;
            const enableAuth = document.getElementById('route-enable-auth').checked;

            const paths = pathsRaw.split(',').map(p => p.trim()).filter(p => p.length > 0);

            await fetchAPI('/admin/v1/routes', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    id: '',
                    service_id: svcId,
                    hosts: [],
                    paths: paths,
                    methods: [],
                    strip_path: stripPath,
                    enable_cache: enableCache,
                    cache_ttl_secs: enableCache ? 60 : null,
                    enable_auth: enableAuth
                })
            });

            toggleModal('modal-add-route');
            refreshData();
        });
    }

    const userForm = document.getElementById('form-user');
    if (userForm) {
        userForm.addEventListener('submit', async (e) => {
            e.preventDefault();
            const username = document.getElementById('user-name').value;
            const password = document.getElementById('user-pass').value;
            const role = document.getElementById('user-role').value;

            const res = await fetchAPI('/admin/v1/users', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ username, password, role })
            });

            if (res && res.status === "created_and_persisted") {
                toggleModal('modal-add-user');
                refreshData();
            } else {
                alert(res?.error || "Failed to create user identity");
            }
        });
    }

    const standaloneForm = document.getElementById('standalone-login-form');
    if (standaloneForm) {
        standaloneForm.addEventListener('submit', async (e) => {
            e.preventDefault();
            const user = document.getElementById('custom-user').value;
            const pass = document.getElementById('custom-pass').value;
            await performLogin(user, pass);
        });
    }

    refreshData();
    setInterval(refreshData, 2500);
});
