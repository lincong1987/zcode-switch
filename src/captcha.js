
import { invoke } from "@tauri-apps/api/core";

const SDK_URL = "https://o.alicdn.com/captcha-frontend/aliyunCaptcha/AliyunCaptcha.js";
const TRACELESS_TIMEOUT = 8000;

const $text = document.getElementById("cap-text");
const $detail = document.getElementById("cap-detail");
const $dot = document.getElementById("cap-dot");
const $btn = document.getElementById("cap-btn");

function status(text, tone = "run") {
  $text.textContent = text;
  $dot.className = "cap-dot" + (tone === "ok" ? " ok" : tone === "err" ? " err" : "");
}

function detail(text) {
  $detail.textContent = text || "";
}

document.addEventListener("securitypolicyviolation", (e) => {
  detail(`CSP 拦截 ${e.violatedDirective} ← ${String(e.blockedURI).slice(0, 70)}`);
});

function loadSdk() {
  return new Promise((resolve, reject) => {
    if (typeof window.initAliyunCaptcha === "function") return resolve();
    const s = document.createElement("script");
    s.src = SDK_URL;
    s.onload = () => resolve();
    s.onerror = () => reject(new Error("验证码组件加载失败，请检查网络"));
    document.head.appendChild(s);
  });
}

let submitted = false;
let region = null;
let tracelessTimer = 0;

async function run() {
  let cfg;
  try {
    cfg = await invoke("claim_captcha_config");
  } catch (e) {
    status("验证配置获取失败", "err");
    detail(String(e));
    return;
  }
  if (!cfg.enabled || !cfg.scene_id) {
    status("验证配置不可用", "err");
    detail("活动可能已结束，请回主窗口刷新后重试");
    return;
  }
  region = cfg.region || null;
  try {
    await loadSdk();
  } catch (e) {
    status(e.message || "验证码组件加载失败", "err");
    return;
  }

  window.AliyunCaptchaConfig = { region: cfg.region, prefix: cfg.prefix };

  status("正在无感验证…");

  const submit = (param) => {
    if (submitted || !param || !param.trim()) return;
    submitted = true;
    clearTimeout(tracelessTimer);
    status("验证通过，正在领取…");
    invoke("claim_captcha_submit", { param, region }).catch((e) => {
      status("领取请求失败", "err");
      detail(String(e));
    });
  };

  const interactive = (why) => {
    clearTimeout(tracelessTimer);
    status("需要人工验证，请点击下方按钮");
    $btn.hidden = false;
    $btn.focus();
    if (why) detail(typeof why === "string" ? why.slice(0, 120) : JSON.stringify(why).slice(0, 120));
  };

  try {
    window.initAliyunCaptcha({
      SceneId: cfg.scene_id,
      mode: "popup",
      language: "zh-CN",
      showErrorTip: false,
      element: "#cap-holder",
      button: "#cap-btn",
      getInstance: (instance) => {
        if (typeof instance.startTracelessVerification === "function") {
          instance.startTracelessVerification();
          tracelessTimer = setTimeout(interactive, TRACELESS_TIMEOUT);
        } else {
          interactive();
        }
      },
      success: (param) => submit(typeof param === "string" ? param : param?.captchaVerifyParam),
      fail: (p) => interactive(p),
      onError: (p) => interactive(p),
    });
  } catch (e) {
    status("验证码初始化失败", "err");
    detail(String(e));
  }
}

$btn.addEventListener("click", () => {
  if (!$btn.hidden) status("请在弹窗中完成验证…");
});

run();
