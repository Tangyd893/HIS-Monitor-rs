// ── HIS-Monitor-rs 运营大屏 JS ──

// ── 状态 ──
let alertsData = [];
let silencesData = [];
let rulesData = [];

// ── 初始化 ──
document.addEventListener('DOMContentLoaded', () => {
  startClock();
  refreshAll();
  // 每 15 秒自动刷新
  setInterval(refreshAll, 15000);
});

// ── 时钟 ──
function startClock() {
  const tick = () => {
    const now = new Date();
    document.getElementById('topbar-clock').textContent =
      now.toLocaleString('zh-CN', { hour12: false });
  };
  tick();
  setInterval(tick, 1000);
}

// ── 全局刷新 ──
async function refreshAll() {
  await Promise.all([
    refreshHealth(),
    refreshAlerts(),
    refreshSilences(),
    refreshRules(),
    refreshMetrics(),
  ]);
}

// ── API 基础封装 ──
async function api(method, path, body) {
  const opts = {
    method,
    headers: { 'Content-Type': 'application/json' },
  };
  if (body !== undefined) opts.body = JSON.stringify(body);
  const resp = await fetch(path, opts);
  const data = await resp.json();
  return { ok: resp.ok, status: resp.status, data };
}

async function apiGet(path) { return api('GET', path); }
async function apiPost(path, body) { return api('POST', path, body); }
async function apiDelete(path) { return api('DELETE', path); }

// ── 健康检查 ──
async function refreshHealth() {
  const el = document.getElementById('topbar-health');
  const apiEl = document.getElementById('health-api');
  try {
    const { ok, data } = await apiGet('/health');
    if (ok && data.status === 'ok') {
      el.className = 'badge badge-ok';
      el.textContent = '● 系统正常';
      apiEl.textContent = '正常';
      apiEl.style.color = 'var(--green)';
    } else {
      el.className = 'badge badge-err';
      el.textContent = '● 异常';
      apiEl.textContent = '异常';
      apiEl.style.color = 'var(--red)';
    }
  } catch {
    el.className = 'badge badge-err';
    el.textContent = '● 离线';
    apiEl.textContent = '离线';
    apiEl.style.color = 'var(--red)';
  }
}

// ── 告警 ──
async function refreshAlerts() {
  const { ok, data } = await apiGet('/api/v1/alerts');
  if (!ok || !data.alerts) {
    alertsData = [];
  } else {
    alertsData = data.alerts;
  }
  document.getElementById('health-active-alerts').textContent =
    alertsData.filter(a => a.status === 'Firing').length;
  renderAlerts();
}

function renderAlerts() {
  const levelFilter = document.getElementById('alert-level-filter').value;
  const statusFilter = document.getElementById('alert-status-filter').value;
  let filtered = alertsData;
  if (levelFilter) filtered = filtered.filter(a => a.level === levelFilter);
  if (statusFilter) filtered = filtered.filter(a => a.status === statusFilter);

  const tbody = document.getElementById('alerts-tbody');
  const countEl = document.getElementById('alert-count');
  countEl.textContent = `(${filtered.length})`;

  if (filtered.length === 0) {
    tbody.innerHTML = '<tr><td colspan="7" class="empty-row">✅ 暂无告警</td></tr>';
    document.getElementById('topbar-alerts').className = 'badge badge-ok';
    document.getElementById('topbar-alerts').textContent = '告警: 0';
    return;
  }

  const firingCount = filtered.filter(a => a.status === 'Firing').length;
  const topbarEl = document.getElementById('topbar-alerts');
  if (firingCount > 0) {
    topbarEl.className = 'badge badge-err';
    topbarEl.textContent = `告警: ${firingCount}`;
  } else {
    topbarEl.className = 'badge badge-ok';
    topbarEl.textContent = `告警: 0 (${filtered.length} 非活跃)`;
  }

  tbody.innerHTML = filtered.map(a => `
    <tr>
      <td><span class="level-tag level-${a.level}">${levelIcon(a.level)} ${a.level}</span></td>
      <td>${esc(a.rule_name)}</td>
      <td>${esc(a.service_name)}</td>
      <td title="${esc(a.description || '')}">${esc(trunc(a.summary, 40))}</td>
      <td>${formatTime(a.fired_at)}</td>
      <td><span class="status-tag status-${a.status}">${statusLabel(a.status)}</span></td>
      <td>
        ${a.status === 'Firing'
          ? `<button class="btn btn-sm btn-ack" onclick="ackAlert('${a.id}')">✓ 确认</button>`
          : '—'}
      </td>
    </tr>
  `).join('');
}

async function ackAlert(id) {
  const { ok, data } = await apiPost(`/api/v1/alerts/${id}/ack`, { comment: '已确认' });
  if (ok) {
    refreshAlerts();
  } else {
    alert(`确认失败: ${data.error || '未知错误'}`);
  }
}

// ── 静默窗口 ──
async function refreshSilences() {
  const { ok, data } = await apiGet('/api/v1/silences');
  silencesData = ok && data.silences ? data.silences : [];
  document.getElementById('health-silences').textContent = silencesData.length;
  renderSilences();
}

function renderSilences() {
  const list = document.getElementById('silences-list');
  if (silencesData.length === 0) {
    list.innerHTML = '<div class="empty-row">暂无静默规则</div>';
    return;
  }
  list.innerHTML = silencesData.map(s => `
    <div class="item-row">
      <div class="item-info">
        <span class="item-name">${esc(s.reason || '无原因')}</span>
        <span class="item-meta">${formatTime(s.starts_at)} → ${formatTime(s.ends_at)}</span>
        ${s.matchers && s.matchers.length > 0
          ? `<span class="item-meta">标签: ${s.matchers.map(m => `${m.key}=${m.value}`).join(', ')}</span>`
          : ''}
      </div>
      <div class="item-actions">
        <button class="btn btn-sm btn-danger" onclick="deleteSilence('${s.id}')">删除</button>
      </div>
    </div>
  `).join('');
}

function showSilenceForm() {
  document.getElementById('silence-modal').style.display = 'flex';
}

async function submitSilence(e) {
  e.preventDefault();
  const form = e.target;
  const body = {
    reason: form.reason.value,
    starts_at: form.starts_at.value + ':00Z',
    ends_at: form.ends_at.value + ':00Z',
    matchers: [],
    min_severity: form.min_severity.value || undefined,
  };
  if (form.label_key.value && form.label_value.value) {
    body.matchers = [{ key: form.label_key.value, value: form.label_value.value }];
  }
  const { ok, data } = await apiPost('/api/v1/silences', body);
  if (ok) {
    closeModal('silence-modal');
    form.reset();
    refreshSilences();
  } else {
    alert(`创建失败: ${data.error || '未知错误'}`);
  }
}

async function deleteSilence(id) {
  if (!confirm('确认删除此静默规则？')) return;
  const { ok } = await apiDelete(`/api/v1/silences/${id}`);
  if (ok) refreshSilences();
}

// ── 告警规则 ──
async function refreshRules() {
  const { ok, data } = await apiGet('/api/v1/rules');
  rulesData = ok && data.rules ? data.rules : [];
  document.getElementById('health-rules').textContent = rulesData.length;
  renderRules();
}

function renderRules() {
  const list = document.getElementById('rules-list');
  if (rulesData.length === 0) {
    list.innerHTML = '<div class="empty-row">暂无规则</div>';
    return;
  }
  list.innerHTML = rulesData.map(r => `
    <div class="item-row">
      <div class="item-info">
        <span class="item-name">${esc(r.name)}</span>
        <span class="item-meta">
          <span class="level-tag level-${r.level}">${levelIcon(r.level)} ${r.level}</span>
          ${r.metric_pattern} ${opSymbol(r.op)} ${r.threshold} · ${r.duration_secs}s
        </span>
      </div>
    </div>
  `).join('');
}

function showRuleForm() {
  document.getElementById('rule-modal').style.display = 'flex';
}

async function submitRule(e) {
  e.preventDefault();
  const form = e.target;
  const body = {
    name: form.name.value,
    metric_pattern: form.metric_pattern.value,
    op: form.op.value,
    threshold: parseFloat(form.threshold.value),
    duration_secs: parseInt(form.duration_secs.value) || 0,
    level: form.level.value,
    summary: form.summary.value,
    description: form.description.value,
    label_matchers: [],
    labels: [],
    group_by: [],
  };
  const { ok, data } = await apiPost('/api/v1/rules', body);
  if (ok) {
    closeModal('rule-modal');
    form.reset();
    refreshRules();
  } else {
    alert(`创建失败: ${data.error || '未知错误'}`);
  }
}

// ── 指标 ──
async function refreshMetrics() {
  try {
    const resp = await fetch('/metrics');
    const text = await resp.text();
    document.getElementById('metrics-raw').textContent =
      text || '(无指标数据)';
  } catch {
    document.getElementById('metrics-raw').textContent = '(获取指标失败)';
  }
}

// ── 工具函数 ──
function esc(s) { return (s || '').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;'); }
function trunc(s, n) { return s && s.length > n ? s.slice(0, n) + '…' : (s || ''); }
function closeModal(id) { document.getElementById(id).style.display = 'none'; }

function formatTime(ts) {
  if (!ts) return '—';
  const d = new Date(ts);
  return d.toLocaleString('zh-CN', { month:'2-digit', day:'2-digit', hour:'2-digit', minute:'2-digit', second:'2-digit' });
}

function levelIcon(l) {
  const m = { Warning: '⚠️', Critical: '🔥', Emergency: '💀' };
  return m[l] || '';
}

function statusLabel(s) {
  const m = { Firing: '告警中', Acked: '已确认', Pending: '待触发', Resolved: '已恢复' };
  return m[s] || s;
}

function opSymbol(op) {
  const m = { Gt: '>', Lt: '<', Gte: '>=', Lte: '<=' };
  return m[op] || op;
}
