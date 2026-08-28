import { invoke } from "@tauri-apps/api/core";
import { esc, toast, openPwModal, installDelegation, dismissSplash } from "./ui.js";
import { ic } from "./icons.js";

const $app = document.getElementById("app");
let state = null;
let autostart = false;
let busy = false;

async function refresh() {
  state = await invoke("get_state");
  autostart = await invoke("autostart_status").catch(() => false);
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

const actions = {
  async refresh() { await refresh(); render(); },

  async toggleAutostart() {
    await guard(async () => {
      const v = await invoke("autostart_set", { enable: !autostart });
      autostart = v;
      toast(v ? "已开启开机自启" : "已关闭开机自启");
      render();
    });
  },

  async toggleBehavior(key) {
    await guard(async () => {
      await invoke("set_behavior", {
        launchAfterSwitch: key === "launch" ? !state.launch_after_switch : null,
        closeToTray: key === "tray" ? !state.close_to_tray : null,
        hotSwitch: key === "hot" ? !state.hot_switch : null,
      });
      await refresh(); render();
      toast("设置已保存");
    });
  },

  async exportAll() {
    await guard(async () => {
      const p = await invoke("export_all_pick_path");
      if (!p.picked) { toast("已取消导出"); return; }
      openPwModal({ mode: "exportAll", path: p.path, count: p.count, onDone: () => actions.refresh() });
    });
  },

  async importFiles() {
    await guard(async () => {
      const p = await invoke("import_pick_files");
      if (!p.picked) return;
      const sealed = p.sealed || [];
      const preErrors = p.errors || [];
      if (sealed.length) {
        openPwModal({ mode: "import", files: sealed, preErrors, onDone: (rep) => actions.finishImport(rep) });
        return;
      }
      actions.finishImport({ added: [], skipped: [], errors: preErrors });
    });
  },

  finishImport(report) {
    if (report.added.length === 0 && report.skipped.length === 0) {
      toast("没有可导入的账号", "err", report.errors.join("；") || undefined);
    } else {
      const parts = [];
      if (report.added.length) parts.push(`导入 ${report.added.length} 个：${report.added.join("、")}`);
      if (report.skipped.length) parts.push(`跳过 ${report.skipped.length} 个（已存在或重复）`);
      if (report.errors.length) parts.push(`失败 ${report.errors.length} 个`);
      toast(parts[0], report.errors.length ? "err" : "ok", parts.slice(1).join("；"));
    }
    refresh().then(render);
  },

  async browsePath() {
    await guard(async () => {
      const r = await invoke("pick_zcode_path");
      if (r.picked) {
        await invoke("set_zcode_path", { path: r.path });
        toast("已更新 ZCode 路径");
        await refresh(); render();
      }
    });
  },

  async savePath() {
    const input = document.querySelector(".settings input.zcode-path");
    if (!input) return;
    await guard(async () => {
      await invoke("set_zcode_path", { path: input.value.trim() });
      toast("已更新 ZCode 路径");
      await refresh(); render();
    });
  },

  async toggleAuthProxy() {
    const input = document.querySelector(".settings input.auth-proxy");
    const url = (input?.value || "").trim() || state.auth_proxy_url || null;
    await guard(async () => {
      await invoke("set_auth_proxy", { on: !state.auth_proxy_on, url });
      await refresh(); render();
      toast(state.auth_proxy_on ? "授权登录代理已开启" : "授权登录代理已关闭", "ok",
        "仅「添加账号」的登录窗口走代理，其余流量不变");
    });
  },

  async saveProxy() {
    const input = document.querySelector(".settings input.auth-proxy");
    if (!input) return;
    await guard(async () => {
      await invoke("set_auth_proxy", { on: state.auth_proxy_on, url: input.value.trim() });
      await refresh(); render();
      toast("代理地址已保存", "ok",
        state.auth_proxy_on ? "已对登录窗口生效" : "开关未开启，暂不生效");
    });
  },
};

const toggle = (on, onclickAttr, label, desc) => `
  <div class="tog-row">
    <div class="tog-info"><div class="tog-label">${label}</div><div class="tog-desc">${desc}</div></div>
    <button class="toggle${on ? " on" : ""}" role="switch" aria-checked="${on}" aria-label="${label}" click="${onclickAttr}">
      <span class="knob"></span>
    </button>
  </div>`;

function render() {
  if (!state) {
    $app.innerHTML = `<div class="loading">LOADING</div>`;
    return;
  }
  const s = state;
  $app.innerHTML = `
    <header class="topbar">
      <div class="wordmark">Z·SWITCH <span class="ver">/ 设置</span></div>
    </header>
    <section class="settings open">
      <label>BEHAVIOR · 行为</label>
      ${toggle(autostart, "actions.toggleAutostart()", "开机自启动", "Windows 登录后自动运行本工具（驻留托盘）")}
      ${toggle(s.launch_after_switch, "actions.toggleBehavior('launch')", "切换后自动启动 ZCode", "切换完成后自动拉起 ZCode")}
      ${toggle(s.close_to_tray, "actions.toggleBehavior('tray')", "关闭窗口时驻留托盘", "点 × 不退出，最小化到系统托盘（仅主窗口）")}
      ${toggle(s.hot_switch, "actions.toggleBehavior('hot')", "热切换（不重启 ZCode）", "默认关：切换要更换设备身份，客户端重启后才认新设备码。开启 = 运行中直接替换登录态，已打开的会话仍用原账号，且客户端可能继续用旧设备码直至重启（自担风险）")}
      <label style="margin-top:14px">AUTH · 授权登录</label>
      ${toggle(s.auth_proxy_on, "actions.toggleAuthProxy()", "授权登录走代理", "z.ai 登录页按出口 IP 分流：国内 IP 只显示手机号登录，邮箱登录需海外出口。开启后仅「添加账号」弹出的登录窗口走此代理；额度查询 / 领取 / 切换等流量不变")}
      <div class="path-line" style="margin-top:6px">
        <input class="zcode-path auth-proxy" type="text" value="${esc(s.auth_proxy_url || "")}"
          placeholder="http://127.0.0.1:7890 或 socks5://127.0.0.1:1080" keydown="onProxyKey(event)">
        <button class="btn-ghost" click="actions.saveProxy()">保存</button>
      </div>
      <label style="margin-top:14px">LIBRARY · 账号库</label>
      <div class="lib-row">
        <button class="btn-ghost has-ic" click="actions.importFiles()">${ic("import", 14)} 导入账号文件</button>
        <button class="btn-ghost has-ic" click="actions.exportAll()" ${s.accounts.length ? "" : "disabled"}>${ic("exportAll", 14)} 导出全部（加密）</button>
      </div>
      <label style="margin-top:14px">ZCODE PATH · ZCode 程序路径</label>
      <div class="path-line">
        <input class="zcode-path" type="text" value="${esc(s.zcode_path)}" placeholder="C:\\Program Files\\ZCode\\ZCode.exe" keydown="onPathKey(event)">
        <button class="btn-ghost" click="actions.browsePath()">浏览</button>
        <button class="btn-ghost" click="actions.savePath()">保存</button>
      </div>
      <div class="hint">切换会同时替换 credentials.json 与 config.json；未入库的当前登录切换前会自动保存，绝不丢号。</div>
    </section>`;
}

window.actions = actions;
window.onPathKey = (e) => { if (e.key === "Enter") actions.savePath(); };
window.onProxyKey = (e) => { if (e.key === "Enter") actions.saveProxy(); };
installDelegation();

(async () => {
  try {
    await refresh();
    render();
    dismissSplash();
  } catch (e) {
    $app.innerHTML = `<div class="loading" style="color:var(--red)">加载失败：${esc(String(e))}</div>`;
    dismissSplash();
  }
})();
