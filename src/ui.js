import { invoke } from "@tauri-apps/api/core";
import { ic } from "./icons.js";

document.addEventListener("contextmenu", (e) => e.preventDefault());

document.addEventListener("keydown", (e) => {
  if (e.key === "F12") e.preventDefault();
  if (e.ctrlKey && e.shiftKey && ["I", "i", "J", "j", "C", "c"].includes(e.key)) e.preventDefault();
  if (e.ctrlKey && ["u", "s"].includes(e.key.toLowerCase())) e.preventDefault();
});

export function esc(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

export function dismissSplash() {
  const el = document.getElementById("splash");
  if (!el) return;
  el.style.opacity = "0";
  setTimeout(() => el.remove(), 220);
}

export function toast(msg, kind = "ok", detail = "") {
  let zone = document.querySelector(".toast-zone");
  if (!zone) {
    zone = document.createElement("div");
    zone.className = "toast-zone";
    document.body.appendChild(zone);
  }
  const t = document.createElement("div");
  t.className = `toast ${kind}`;
  t.innerHTML = `<span class="t-ic">${ic(kind === "ok" ? "check" : "alert", 16)}</span>
    <span class="t-body">${esc(msg)}${detail ? `<span class="detail">${esc(detail)}</span>` : ""}</span>`;
  zone.appendChild(t);
  setTimeout(() => t.remove(), detail ? 5200 : 3200);
}

function runAttr(expr, event) {
  try {
    const open = expr.indexOf("(");
    if (open < 0 || !expr.trimEnd().endsWith(")")) return;
    let fn = window;
    for (const seg of expr.slice(0, open).trim().split(".")) fn = fn?.[seg];
    if (typeof fn !== "function") return;
    const src = expr.slice(open + 1, expr.trimEnd().length - 1).trim();
    const args = src
      ? src.split(/\s*,\s*/).map((a) => {
          if (a === "event") return event;
          const m = a.match(/^'([^']*)'$/);
          if (m) return m[1];
          return JSON.parse(a);
        })
      : [];
    fn(...args);
  } catch (e) {
    console.warn("attr handler error:", expr, e);
  }
}

export function installDelegation() {
  document.addEventListener("click", (e) => {
    const el = e.target.closest("[click]");
    if (!el) return;
    e.preventDefault();
    runAttr(el.getAttribute("click") || "", e);
  });
  document.addEventListener("keydown", (e) => {
    const el = e.target.closest("[keydown]");
    if (!el) return;
    runAttr(el.getAttribute("keydown") || "", e);
  });
  document.addEventListener("blur", (e) => {
    const el = e.target.closest("[blur]");
    if (!el) return;
    runAttr(el.getAttribute("blur") || "", e);
  }, true);
}

export function openConfirmModal(m) {
  document.querySelector(".cm-mask")?.remove();
  const kind = m.kind || "plain";
  const mask = document.createElement("div");
  mask.className = "cm-mask";
  mask.innerHTML = `
    <div class="cm-panel" role="alertdialog" aria-modal="true" aria-label="${esc(m.title)}">
      <div class="cm-strip ${kind}"></div>
      <div class="cm-head">
        <span class="cm-ic ${kind}">${ic(m.icon || "alert", 21)}</span>
        <div class="cm-title">${esc(m.title)}</div>
      </div>
      ${m.desc ? `<div class="cm-desc">${m.desc}</div>` : ""}
      <div class="cm-actions">
        <button class="btn-ghost cm-no">${esc(m.noLabel || "取消")}</button>
        <button class="cm-yes ${kind}">${esc(m.yesLabel || "确认")}</button>
      </div>
    </div>`;
  document.body.appendChild(mask);

  const panel = mask.querySelector(".cm-panel");
  const yes = mask.querySelector(".cm-yes");
  const no = mask.querySelector(".cm-no");
  const close = () => {
    document.removeEventListener("keydown", mask._key);
    mask.remove();
  };
  panel.addEventListener("click", (e) => e.stopPropagation());
  mask.addEventListener("click", close);
  no.addEventListener("click", close);
  yes.addEventListener("click", async () => {
    yes.disabled = true; no.disabled = true;
    try { await m.onYes?.(); } finally { close(); }
  });
  mask._key = (e) => {
    if (e.key === "Escape") close();
    if (e.key === "Enter" && e.target === document.body && !yes.disabled) yes.click();
  };
  document.addEventListener("keydown", mask._key);
  (m.focusNo || kind === "danger" ? no : yes).focus();
}

export function openPwModal(m) {
  document.querySelector(".pw-mask")?.remove();
  const isExport = m.mode === "export" || m.mode === "exportAll";
  const title = m.mode === "export" ? `导出「${esc(m.name)}」`
    : m.mode === "exportAll" ? `导出全部（${m.count} 个账号）`
    : `导入 ${m.files.length} 个加密文件`;
  const sub = isExport
    ? `保存到 ${esc(m.path)}<br>文件含登录凭据（credentials + config 捆包），<b>由你自设密码加密</b>，密码丢失无法找回`
    : `${m.files.map(([n]) => esc(n)).join("、")}<br>输入导出时设置的密码`;

  const mask = document.createElement("div");
  mask.className = "pw-mask";
  mask.innerHTML = `
    <div class="pw-panel">
      <div class="pw-title">${title}</div>
      <div class="pw-sub">${sub}</div>
      <input class="pw-input" type="password" id="pw1" placeholder="${isExport ? "设置密码（至少 6 位）" : "密码"}" autocomplete="off">
      ${isExport ? `<input class="pw-input" type="password" id="pw2" placeholder="再输入一次确认" autocomplete="off">` : ""}
      <div class="pw-err"></div>
      <div class="pw-actions">
        <button class="btn-ghost pw-cancel">取消</button>
        <button class="btn-primary pw-go has-ic">${isExport ? ic("lock", 14) + " 加密导出" : ic("lockOpen", 14) + " 解密导入"}</button>
      </div>
    </div>`;
  document.body.appendChild(mask);

  const panel = mask.querySelector(".pw-panel");
  const errEl = mask.querySelector(".pw-err");
  const go = mask.querySelector(".pw-go");
  const input1 = mask.querySelector("#pw1");
  const input2 = mask.querySelector("#pw2");
  panel.addEventListener("click", (e) => e.stopPropagation());
  mask.addEventListener("click", () => mask.remove());
  mask.querySelectorAll(".pw-cancel").forEach((b) => b.addEventListener("click", () => mask.remove()));
  const key = (e) => {
    if (e.key === "Escape") mask.remove();
    if (e.key === "Enter") confirm();
  };
  [input1, input2].forEach((i) => i?.addEventListener("keydown", key));
  input1.focus();

  async function confirm() {
    const pw1 = input1?.value || "";
    const pw2 = input2?.value;
    if (pw1.length < 6) return (errEl.textContent = "密码至少 6 位");
    if (pw2 !== undefined && pw2 !== null && pw1 !== pw2) return (errEl.textContent = "两次输入不一致");
    go.disabled = true;
    try {
      if (m.mode === "export") {
        const r = await invoke("export_finalize", { path: m.path, id: m.id, password: pw1 });
        toast(`已加密导出到 ${r.path}`, "ok", "凭据捆包 + 口令加密（记得密码）；.zsb 可在任意一台机器导入");
        m.onDone?.(r);
      } else if (m.mode === "exportAll") {
        const r = await invoke("export_all_finalize", { path: m.path, password: pw1 });
        toast(`已加密导出 ${r.count} 个账号`, "ok", r.path);
        m.onDone?.(r);
      } else {
        let report = await invoke("import_sealed", { files: m.files, password: pw1 });
        if (m.preErrors?.length) {
          report.errors = [...m.preErrors, ...(report.errors || [])];
        }
        const wrongPw = (report.errors || []).some((e) => String(e).includes("密码错误"));
        if (wrongPw) { errEl.textContent = "密码错误，请重试"; go.disabled = false; return; }
        m.onDone?.(report);
      }
      mask.remove();
    } catch (e) {
      errEl.textContent = typeof e === "string" ? e : String(e);
      go.disabled = false;
    }
  }
  go.addEventListener("click", confirm);
}

export function openProviderModal(m) {
  document.querySelector(".pv-mask")?.remove();
  const mask = document.createElement("div");
  mask.className = "pv-mask";
  mask.innerHTML = `
    <div class="pv-panel">
      <div class="pv-title">添加账号 · 选择登录方式</div>
      <div class="pv-sub">在弹出的窗口里自行登录（账号密码 / 扫码），<br>登录完成后自动存入账号库，<b>不影响当前正在使用的登录</b></div>
      <div class="pv-list">
        ${(m.providers || []).map((p) => `
          <button class="pv-item" data-id="${esc(p.id)}">
            <span class="pv-name">${esc(p.display)}</span>
            <span class="pv-arrow">→</span>
          </button>`).join("")}
      </div>
      <div class="pv-actions"><button class="btn-ghost pv-cancel">取消</button></div>
    </div>`;
  document.body.appendChild(mask);
  const onKey = (e) => { if (e.key === "Escape") close(); };
  const close = () => { mask.remove(); document.removeEventListener("keydown", onKey); };
  mask.querySelectorAll(".pv-item").forEach((b) => {
    b.addEventListener("click", () => { close(); m.onPick?.(b.dataset.id); });
  });
  mask.querySelector(".pv-cancel").addEventListener("click", close);
  document.addEventListener("keydown", onKey);
  const first = mask.querySelector(".pv-item");
  if (first) first.focus();
}
