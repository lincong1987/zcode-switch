import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { esc, toast, openPwModal, openConfirmModal, openProviderModal, installDelegation, dismissSplash } from "./ui.js";
import { ic } from "./icons.js";

const $app = document.getElementById("app");
let state = null;
let renaming = null;
let busy = false;
let acctQuota = {};
let claimable = {};
let claimAllRunning = false;

const NOTCH_COLORS = ["var(--notch-1)", "var(--notch-2)", "var(--notch-3)", "var(--notch-4)", "var(--notch-5)", "var(--notch-6)"];
function notchColor(id) {
  let h = 0;
  for (const c of id) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return NOTCH_COLORS[h % NOTCH_COLORS.length];
}
function fmtNum(v) {
  if (v == null) return "未知";
  const n = Number(v);
  if (!isFinite(n)) return "未知";
  if (Math.abs(n) >= 1e8) return (n / 1e8).toFixed(2) + " 亿";
  if (Math.abs(n) >= 1e4) return (n / 1e4).toFixed(2) + " 万";
  return n.toLocaleString("zh-CN", { maximumFractionDigits: 2 });
}
function idLabel(id) {
  if (!id) return null;
  return id.display_name || id.username || id.email || null;
}

async function refresh() {
  state = await invoke("get_state");
}

function uiLocked() {
  return renaming !== null;
}

async function guard(fn) {
  if (busy) return;
  busy = true;
  try {
    await fn();
  } catch (e) {
    toast(typeof e === "string" ? e : String(e), "err");
  } finally {
    busy = false;
  }
}

async function loadAcctQuota(id) {
  const cur = acctQuota[id] || {};
  if (cur.busy) return;
  acctQuota[id] = { busy: true };
  if (!uiLocked()) render();
  try {
    const data = await invoke("get_account_quota", { id });
    acctQuota[id] = { data, err: null, busy: false };
  } catch (e) {
    acctQuota[id] = { data: null, err: typeof e === "string" ? e : String(e), busy: false };
  }
  if (!uiLocked()) render();
}

async function loadClaimPreview(id) {
  const cur = claimable[id] || {};
  if (cur.busy) return;
  claimable[id] = { plans: cur.plans || [], busy: true };
  try {
    const plans = await invoke("claim_preview", { id });
    claimable[id] = { plans: plans || [], err: null, busy: false };
  } catch (e) {
    claimable[id] = { plans: cur.plans || [], err: String(e), busy: false };
  }
}

let claimWaiter = null;
function waitForClaimResult(accountId, timeoutMs = 90000) {
  return new Promise((resolve) => {
    let done = false;
    const finish = (v) => { if (!done) { done = true; claimWaiter = null; clearTimeout(t); resolve(v); } };
    const t = setTimeout(() => finish(null), timeoutMs);
    claimWaiter = { accountId, finish };
  });
}

const actions = {
  async refresh() { await refresh(); render(); },

  async capture() {
    await guard(async () => {
      const r = await invoke("capture_current", { name: null });
      toast(`已保存当前登录为「${r.name}」`, "ok", "之后可随时切回这个账号");
      await refresh(); render();
      enrollAccounts();
    });
  },

  async rename(id) {
    renaming = id; render();
    const input = document.querySelector(`.row[data-id="${id}"] .rename-input`);
    if (input) { input.focus(); input.select(); }
  },

  async doRename(id) {
    const input = document.querySelector(`.row[data-id="${id}"] .rename-input`);
    const name = (input?.value || "").trim();
    if (!name) return;
    window.__renameSaving = true;
    clearTimeout(window.__renameBlurTimer);
    await guard(async () => {
      const r = await invoke("rename_account", { id, name });
      toast(`已重命名为「${r.name}」`);
      renaming = null;
      await refresh(); render();
    }).finally(() => { window.__renameSaving = false; });
  },

  cancelRename() { renaming = null; render(); },

  deferCancelRename(id) {
    clearTimeout(window.__renameBlurTimer);
    window.__renameBlurTimer = setTimeout(() => {
      if (renaming === id && !window.__renameSaving) actions.cancelRename();
    }, 180);
  },

  async delete(id) {
    const a = state?.accounts.find((x) => x.id === id);
    if (!a) return;
    openConfirmModal({
      kind: "danger",
      icon: "x",
      title: `删除「${a.name}」？`,
      desc: "只删除账号库里的存档，不影响当前登录",
      yesLabel: "删除",
      onYes: () => actions.doDelete(id),
    });
  },

  async doDelete(id) {
    await guard(async () => {
      await invoke("delete_account", { id });
      toast("账号已删除");
      await refresh(); render();
    });
  },

  askSwitch(id) {
    if (state.zcode_running && !state.hot_switch) {
      const a = state.accounts.find((x) => x.id === id);
      if (!a) return;
      openConfirmModal({
        kind: "warn",
        icon: "swap",
        title: `切换到「${a.name}」？`,
        desc: `<span class="warn-line">ZCode 正在运行：将自动关闭并${state.launch_after_switch ? "在切换后重启" : "不重启"}</span>（可在设置里改）`,
        yesLabel: "强制切换",
        onYes: () => actions.doSwitch(id, true),
      });
    } else {
      actions.doSwitch(id, false);
    }
  },

  async doSwitch(id, force) {
    await guard(async () => {
      const restart = state.launch_after_switch;
      const r = await invoke("switch_to", { id, force, restart });
      if (r.already_active) {
        toast(`「${r.name}」已是当前登录`, "ok");
      } else {
        const bits = [];
        if (r.hot) bits.push("热切换 · ZCode 未重启");
        if (r.killed) bits.push("已关闭运行中的 ZCode");
        if (r.preserved_as) bits.push(`原登录自动保存为「${r.preserved_as}」`);
        if (r.launched) bits.push("已重新启动 ZCode");
        if (r.config_stale) bits.push("注意：该账号无 config 快照，沿用现有 config.json（额度可能显示别的账号，登录后可点↻同步）");
        toast(`已切换到「${r.name}」`, r.config_stale ? "warn" : "ok", bits.join("；"));
      }
      await refresh(); render();
      pokeAccount(id);
    });
  },

  async updateFromLive(id) {
    await guard(async () => {
      const r = await invoke("update_account_from_live", { id });
      toast(`「${r.name}」已同步当前登录`, "ok", "token 刷新后的最新状态已存档");
      await refresh(); render();
    });
  },

  async exportOne(id) {
    await guard(async () => {
      const p = await invoke("export_pick_path", { id });
      if (!p.picked) { toast("已取消导出"); return; }
      openPwModal({ mode: "export", id, path: p.path, name: p.name });
    });
  },

  async launch() {
    await guard(async () => {
      await invoke("launch_zcode");
      toast("正在启动 ZCode…");
      setTimeout(() => actions.refresh(), 2500);
    });
  },

  askKill() {
    openConfirmModal({
      kind: "danger",
      icon: "power",
      title: "关闭 ZCode？",
      yesLabel: "关闭",
      onYes: () => actions.doKill(),
    });
  },

  async doKill() {
    await guard(async () => {
      await invoke("kill_zcode");
      toast("已关闭 ZCode");
      await refresh(); render();
    });
  },

  async openSettings() {
    try { await invoke("open_settings"); }
    catch (e) { toast(typeof e === "string" ? e : String(e), "err"); }
  },
  acctQuota(id) {
    const dueAt = quotaDue[id];
    loadAcctQuota(id).then(() => {
      if (quotaDue[id] === dueAt) scheduleNext(id);
    });
  },

  async addAccount() {
    let providers;
    try { providers = await invoke("oauth_providers"); }
    catch (e) { toast(typeof e === "string" ? e : String(e), "err"); return; }
    openProviderModal({
      providers,
      onPick: async (id) => {
        try {
          await invoke("oauth_begin", { provider: id });
          toast("登录窗口已打开", "ok", "登录完成后账号会自动入库");
        } catch (e) {
          toast(typeof e === "string" ? e : String(e), "err");
        }
      },
    });
  },

  async claim(id) {
    if (claimAllRunning) { toast("批量领取进行中，请等待完成", "warn"); return; }
    const plans = claimable[id]?.plans || [];
    const plan = plans[0];
    if (!plan) { toast("该账号暂无可领取的套餐", "warn"); return; }
    try {
      await invoke("claim_start", { id, planId: plan.plan_id });
      toast(`正在为「${plan.name || plan.plan_id}」完成安全验证…`, "ok", "无感验证通常 2-3 秒");
      const r = await waitForClaimResult(id);
      if (!r) toast("验证超时或已取消", "warn");
    } catch (e) {
      toast(typeof e === "string" ? e : String(e), "err");
    }
  },

  async claimAll() {
    const ids = (state?.accounts || [])
      .map((a) => a.id)
      .filter((id) => (claimable[id]?.plans || []).length > 0);
    if (!ids.length) { toast("没有可领取的账号", "warn"); return; }
    if (claimAllRunning) return;
    claimAllRunning = true;
    try {
      for (let i = 0; i < ids.length; i++) {
        const id = ids[i];
        const plan = claimable[id].plans[0];
        const name = state.accounts.find((a) => a.id === id)?.name || id;
        try {
          await invoke("claim_start", { id, planId: plan.plan_id });
        } catch (e) {
          toast(`「${name}」${String(e)}`, "err");
          continue;
        }
        const r = await waitForClaimResult(id, 120000);
        if (!r) {
          toast(`「${name}」验证超时，已跳过`, "warn");
          await invoke("claim_cancel").catch(() => {});
        }
        if (i < ids.length - 1) await new Promise((res) => setTimeout(res, 1200));
      }
    } finally {
      claimAllRunning = false;
    }
  },
};

function quotaBarHtml(pct) {
  const used = pct == null ? null : Math.min(100, Math.max(0, pct));
  const remaining = used == null ? null : 100 - used;
  const danger = used != null && used >= 90 ? " danger" : used != null && used >= 70 ? " warn" : "";
  const txt = remaining == null ? "--" : remaining.toFixed(0) + "%";
  const txtCls = (remaining ?? 100) >= 58 ? " in-fill" : "";
  return `<div class="qbar${danger}"><div class="qbar-fill" style="width:${remaining ?? 100}%"></div><span class="qbar-pct${txtCls}">${txt}</span></div>`;
}

function shortWinLabel(name) {
  const m = name.match(/[（(]每\s*([^）)]+)[）)]/);
  if (m) return "每" + m[1].replace(/^每/, "");
  if (name.includes("使用时长")) return "月度";
  if (name.includes("提示次数")) return "次数";
  return name;
}

function winRowHtml(it, cls = "") {
  return `
  <div class="q-win${cls}">
    <span class="q-win-label">${esc(shortWinLabel(it.name))}</span>
    ${quotaBarHtml(it.percent_used)}
    <span class="q-win-reset" title="${esc(it.period_end || "")}">${it.period_end ? esc(it.period_end) : ""}</span>
  </div>`;
}

function fmtTokens(n) {
  if (n == null) return "";
  if (n >= 1e8) return (n / 1e8).toFixed(n % 1e8 === 0 ? 0 : 1) + "亿";
  if (n >= 1e6) return (n / 1e6).toFixed(n % 1e6 === 0 ? 0 : 1) + "M";
  if (n >= 1e3) return Math.round(n / 1e3) + "K";
  return String(Math.round(n));
}
function balRowHtml(it) {
  const rem = it.total != null && it.remaining != null ? `${fmtTokens(it.remaining)}/${fmtTokens(it.total)}` : "";
  const label = it.name.replace(/^GLM-?/i, "");
  return `
  <div class="q-win mini">
    <span class="q-win-label" title="${esc(it.name)}">${esc(label)}</span>
    ${quotaBarHtml(it.percent_used)}
    <span class="q-win-reset">${esc(rem)}</span>
  </div>`;
}

function tierChipHtml(t) {
  const s = String(t).toLowerCase();
  let label, cls;
  if (s.includes("max")) { label = "Max"; cls = "max"; }
  else if (s.includes("pro")) { label = "Pro"; cls = "pro"; }
  else if (s.includes("lite")) { label = "Lite"; cls = "lite"; }
  else if (s.includes("trial") || s.includes("体验") || s.includes("start")) { label = "体验"; cls = "trial"; }
  else { label = t; cls = "other"; }
  return `<span class="tier-b ${cls}">${esc(label)}</span>`;
}
function tierBadgeFor(id) {
  const q = acctQuota[id];
  if (!q?.data) return "";
  const tiers = [...new Set((q.data.plans || []).map((p) => p.tier).filter(Boolean))];
  const list = (tiers.length ? tiers : q.data.plan_tier ? [q.data.plan_tier] : []).slice(0, 2);
  if (!list.length) return `<span class="tier-b free">Free</span>`;
  return list.map(tierChipHtml).join("");
}

function claimStripHtml(id) {
  const c = claimable[id];
  const plan = c?.plans?.[0];
  if (!plan) return "";
  const grants = (plan.grants || []).slice(0, 1).join("、");
  const label = plan.name || plan.plan_id;
  return `
  <div class="claim-strip" title="${esc(plan.description || label)}">
    ${ic("gift", 15)}
    <span class="claim-name">${esc(label)}</span>
    ${grants ? `<span class="claim-grants">${esc(grants)}</span>` : ""}
    <button class="btn-claim has-ic" click="actions.claim('${id}')" ${claimAllRunning ? "disabled" : ""}>${ic("gift", 13)} 领取</button>
  </div>`;
}

function slotRowsHtml(items) {
  const list = items || [];
  const isWin = (it) => it.name.includes("提示次数") || it.name.includes("使用时长");
  const wins = list.filter(isWin);
  const best = new Map();
  for (const it of list) {
    if (isWin(it)) continue;
    const cur = best.get(it.name);
    if (!cur || (it.total || 0) > (cur.total || 0)) best.set(it.name, it);
  }
  const pools = [...best.values()].sort((a, b) => (b.total || 0) - (a.total || 0));
  return [
    ...wins.map((it) => winRowHtml(it, " mini")),
    ...pools.map(balRowHtml),
  ].join("");
}

function planGroupHtml(p) {
  const label = p.name || p.tier || "";
  return `
  <div class="plan-grp">
    <div class="pg-head">
      ${p.tier ? tierChipHtml(p.tier) : ""}
      <span class="pg-name" title="${esc(label)}">${esc(label)}</span>
      ${p.expire ? `<span class="pg-exp" title="有效期至 ${esc(p.expire)}">至 ${esc(p.expire)}</span>` : ""}
    </div>
    ${slotRowsHtml(p.items)}
  </div>`;
}

function acctQuotaSlot(id) {
  const strip = claimStripHtml(id);
  const q = acctQuota[id];
  let inner = "";
  if (q?.busy) {
    inner = `<span class="aq-loading">查询中…</span>`;
  } else if (q?.err) {
    const msg = q.err.length > 46 ? q.err.slice(0, 46) + "…" : q.err;
    inner = `<span class="aq-err">${esc(msg)}</span>`;
  } else if (q?.data) {
    const plans = q.data.plans || [];
    if (plans.length >= 2) {
      inner = plans.map(planGroupHtml).join("");
    } else {
      const items = q.data.items || [];
      const wins = items.filter((it) => it.name.includes("提示次数"));
      if (wins.length) {
        inner = wins.map((it) => winRowHtml(it, " mini")).join("");
      } else {
        inner = slotRowsHtml(items);
      }
    }
  }
  if (!strip && !inner) return `<div class="row-quota-slot"></div>`;
  return `<div class="row-quota-slot">${strip}${inner}</div>`;
}

function render() {
  if (!state) {
    $app.innerHTML = `<div class="loading">LOADING</div>`;
    return;
  }
  const s = state;
  const active = s.accounts.find((a) => a.is_active) || null;
  const unsaved = s.live_logged_in && !active;

  const dotCls = s.zcode_running ? "run" : s.live_logged_in ? "" : "off";
  const statusText = s.zcode_running
    ? "ZCode 运行中"
    : s.live_logged_in
      ? unsaved ? "未保存的登录" : "可安全切换"
      : "未登录";

  const rows = s.accounts.map((a) => {
    const isActive = a.is_active;
    if (renaming === a.id) {      return `
      <div class="row${isActive ? " active" : ""}" data-id="${a.id}">
        <span class="notch" style="background:${notchColor(a.id)}"></span>
        <div class="row-main">
          <input class="rename-input" value="${esc(a.name)}" maxlength="40"
            keydown="onRenameKey(event,'${a.id}')" blur="actions.deferCancelRename('${a.id}')">
          <div class="row-meta">回车保存 · Esc 取消</div>
        </div>
        <div class="row-actions">
          <button class="btn-ghost" style="padding:4px 10px" click="actions.doRename('${a.id}')">保存</button>
          <button class="btn-ghost" style="padding:4px 10px" click="actions.cancelRename()">取消</button>
        </div>
      </div>`;
    }
    const ident = [a.identity?.username, a.identity?.email].filter(Boolean).join(" · ");
    const q = acctQuota[a.id];
    let meta = "";
    if (!a.has_config) meta += `<span class="no-cfg">无 config 快照（切换后沿用现有 config）</span>`;
    if (q?.data?.plan_expire) {
        const days = Math.ceil((new Date(q.data.plan_expire + "T23:59:59") - Date.now()) / 86400000);
        const cls = days <= 7 ? " warn-line" : "";
        meta += `<span class="${cls.trim()}">有效期至 ${esc(q.data.plan_expire)}</span>`;
    }
    if (ident) meta += `${meta ? " · " : ""}${esc(ident)}`;
    return `
    <div class="row${isActive ? " active" : ""}" data-id="${a.id}">
      <div class="row-top">
        <span class="notch" style="background:${notchColor(a.id)}"></span>
        <div class="row-main">
          <div class="row-name">${esc(a.name)}${tierBadgeFor(a.id)}${isActive ? '<span class="tag-use">使用中</span>' : ""}</div>
          <div class="row-meta">${meta}</div>
        </div>
        <div class="row-actions">
          <button class="icon-btn" title="查额度" aria-label="查额度" click="actions.acctQuota('${a.id}')">${ic("gauge", 16)}</button>
          <button class="icon-btn" title="重命名" aria-label="重命名" click="actions.rename('${a.id}')">${ic("pen", 16)}</button>
          <button class="icon-btn" title="导出" aria-label="导出" click="actions.exportOne('${a.id}')">${ic("export", 16)}</button>
          <button class="icon-btn danger" title="删除" aria-label="删除" click="actions.delete('${a.id}')">${ic("x", 16)}</button>
          <button class="btn-switch has-ic" click="actions.askSwitch('${a.id}')" ${isActive ? "disabled" : ""}>
            ${isActive ? "当前" : ic("swap", 14) + " 切换"}
          </button>
        </div>
      </div>
      ${acctQuotaSlot(a.id)}
    </div>`;
  }).join("");

  const listHtml = s.accounts.length === 0
    ? `<div class="empty">
         <div class="glyph">${ic("empty", 34)}</div>
         账号库为空<br>
         先用下方 <b>保存当前登录</b> 存入第一个账号，或<b>导入</b>账号文件
       </div>`
    : rows;

  const claimableCount = s.accounts.filter((a) => (claimable[a.id]?.plans || []).length > 0).length;

  $app.innerHTML = `
    <header class="topbar">
      <div class="wordmark">Z·SWITCH</div>
      <div class="top-status${unsaved ? " unsaved" : ""}">
        <span class="status-dot ${dotCls}"></span>
        <span class="status-text">${esc(statusText)}</span>
      </div>
    </header>

    <section class="toolbar">
      <button class="btn-primary has-ic${unsaved ? " attention" : ""}" click="actions.capture()" ${!s.live_logged_in || active ? "disabled" : ""}
        title="${active ? `当前登录已是存档「${esc(active.name)}」，无需重复保存` : ""}">
        ${ic("capture", 16)} 保存当前登录
      </button>
      ${claimableCount > 0
        ? `<button class="btn-ghost has-ic claim-all" click="actions.claimAll()" ${claimAllRunning ? "disabled" : ""}
            title="逐账号完成安全验证并领取（每账号限领一次）">${ic("gift", 16)} 全部领取${claimableCount > 1 ? ` (${claimableCount})` : ""}</button>`
        : ""}
      <button class="btn-ghost has-ic" click="actions.addAccount()" title="OAuth 登录新账号入库，不影响当前登录">${ic("userPlus", 16)} 添加账号</button>
      ${s.zcode_running
        ? `<button class="btn-ghost has-ic" click="actions.askKill()" title="关闭 ZCode">${ic("power", 16)} 关闭 ZCode</button>`
        : `<button class="btn-ghost has-ic" click="actions.launch()" ${s.zcode_path_ok ? "" : "disabled"}>${ic("play", 14)} 启动 ZCode</button>`}
      <span class="tb-spacer"></span>
      <button class="btn-ghost tb-gear has-ic" click="actions.openSettings()" aria-label="设置" title="设置">${ic("sliders", 16)}</button>
    </section>

    <div class="section-head">
      <h2>ACCOUNTS · 账号库</h2>
      <span class="count">${s.accounts.length} 个存档</span>
    </div>

    <main class="list">${listHtml}</main>
  `;
}

window.actions = actions;
window.onRenameKey = (e, id) => {
  if (e.key === "Enter") actions.doRename(id);
  if (e.key === "Escape") actions.cancelRename();
};
installDelegation();

listen("tray-action", (ev) => {
  const p = ev.payload || {};
  if (p.action === "capture" && p.ok) toast(`已保存当前登录为「${p.result.name}」`);
  else if (!p.ok && p.error) toast(p.error, "err");
  refresh().then(() => { if (!uiLocked()) { render(); enrollAccounts(); } }).catch(() => {});
});

listen("claim://result", (ev) => {
  const p = ev.payload || {};
  if (claimWaiter && claimWaiter.accountId === p.accountId) claimWaiter.finish(p);
  if (p.ok === false) {
    toast(`「${p.accountName}」领取失败：${p.message || "未知错误"}`, "err");
  } else {
    const bits = [];
    if (p.startsAt) bits.push(`额度将于 ${new Date(p.startsAt).toLocaleString("zh-CN", { hour12: false })} 生效`);
    if (p.endsAt) bits.push(`有效期至 ${new Date(p.endsAt).toLocaleString("zh-CN", { hour12: false })}`);
    toast(`「${p.accountName}」已领取「${p.planName}」`, "ok", bits.join("；"));
  }
  if (p.accountId) {
    loadAcctQuota(p.accountId);
    loadClaimPreview(p.accountId).then(() => { if (!uiLocked()) render(); });
    scheduleNext(p.accountId);
  }
});

listen("oauth://done", (ev) => {
  const p = ev.payload || {};
  if (p.ok === false) {
    toast(`添加账号失败：${p.error || "未知错误"}`, "err");
    return;
  }
  if (p.duplicate) {
    toast(`该登录已在账号库中：「${p.name}」`, "warn", "同一账号无需重复添加；要换号请先在登录页退出登录");
    return;
  }
  toast(`已添加「${p.name}」`, "ok", "不影响当前登录；需要时在列表里切换");
  refresh().then(() => { if (!uiLocked()) { render(); enrollAccounts(); } }).catch(() => {});
});

listen("state-changed", () => {
  refresh().then(() => { if (!uiLocked()) render(); }).catch(() => {});
});

const SWEEP_PERIOD = 5 * 60 * 1000;
const SWEEP_JITTER = 0.2;
const TICK_MS = 8000;
let quotaDue = {};
let ticking = false;

function scheduleNext(id, base = Date.now()) {
  const jitter = 1 + (Math.random() * 2 - 1) * SWEEP_JITTER;
  quotaDue[id] = base + Math.round(SWEEP_PERIOD * jitter);
}
function enrollAccounts() {
  const live = new Set((state?.accounts || []).map((a) => a.id));
  for (const id of live) if (!(id in quotaDue)) quotaDue[id] = Date.now();
  for (const id of Object.keys(quotaDue)) if (!live.has(id)) delete quotaDue[id];
}
function pokeAccount(id) { if (id) quotaDue[id] = Date.now(); }

async function sweepTick() {
  if (ticking) return;
  enrollAccounts();
  const now = Date.now();
  const due = (state?.accounts || []).find(
    (a) => (quotaDue[a.id] ?? Infinity) <= now && !acctQuota[a.id]?.busy && !claimable[a.id]?.busy,
  );
  if (!due) return;
  ticking = true;
  const dueAt = quotaDue[due.id];
  try {
    await loadAcctQuota(due.id);
    await loadClaimPreview(due.id);
    if (quotaDue[due.id] === dueAt) scheduleNext(due.id);
    if (!uiLocked()) render();
  } finally {
    ticking = false;
  }
}

(async () => {
  try {
    await refresh();
    render();
    await invoke("reveal_main");
    setTimeout(dismissSplash, 350);
    enrollAccounts();
    sweepTick();
    setInterval(() => {
      invoke("get_state").then((s) => { state = s; enrollAccounts(); if (!uiLocked()) render(); }).catch(() => {});
    }, 5000);
    setInterval(sweepTick, TICK_MS);
  } catch (e) {
    $app.innerHTML = `<div class="loading" style="color:var(--red)">加载失败：${esc(String(e))}</div>`;
    invoke("reveal_main").catch(() => {});
    dismissSplash();
  }
})();
