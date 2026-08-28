# Z·SWITCH (zcode-switch)

Tauri 2 桌面工具：在多个 ZCode 账号之间一键切换，自动显示额度。只换登录身份——项目、会话、设置全部共用不动。

## 功能

- **保存 / 切换账号**：快照 credentials + config 双文件，原子替换；切换前自动保全当前登录，绝不丢号
- **添加账号**：工具内 OAuth 登录新号（BigModel / z.ai 双入口），全程不动当前登录
- **额度展示**：账号行内联显示套餐额度与重置时间，多套餐分组、错峰轮询
- **活动领取**：可领套餐一键领取（GUI 验证码）
- **加密导入导出**：`.zsb` 捆绑包，PBKDF2(100k) + AES-256-GCM 口令加密
- **托盘 / 开机自启 / CLI 自动化**

## 安全设计

- 本地优先：所有数据在本地，无遥测、无远端存储
- WebView CSP：`script-src 'self'`，无 eval、无内联脚本
- 文件写入走临时文件 + 原子 rename；账号 id 白名单防路径穿越
- 凭据只在本地解密，导出文件凭口令加密

## CLI

```
zcode-switch.exe --cli state|list
zcode-switch.exe --cli quota [--id <账号id>]
zcode-switch.exe --cli claim-preview [--id <账号id>]
zcode-switch.exe --cli capture [--name 名称]
zcode-switch.exe --cli switch --id <id> [--force] [--restart|--no-restart] [--hot <bool>|--no-hot]
zcode-switch.exe --cli kill
zcode-switch.exe --cli export --id <id> --out <a.zsb>
zcode-switch.exe --cli export-all --out <all.zsb>
zcode-switch.exe --cli import --file <file.zsb>
zcode-switch.exe --cli rename|delete|update|behavior|setpath|launch
```

CLI 密码（export / import）：优先环境变量 `ZSW_PASSWORD`，也可 `--password <密码>`。

## 构建

```bash
npm install
npm run tauri build
```

Windows 优先（路径探测 / 进程管理 / 托盘均为 Win32 语义）。

## License

MIT
