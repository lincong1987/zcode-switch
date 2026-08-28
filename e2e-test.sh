#!/usr/bin/env bash
set -u
UP_WIN="${USERPROFILE:?需要 USERPROFILE 环境变量}"
UP_MSYS="$(cygpath -u "$UP_WIN")"
EXE="${ZSWITCH_EXE:-$UP_WIN\\zswitch-build-target\\release\\zcode-switch.exe}"
SB_WIN="$UP_WIN\\AppData\\Local\\Temp\\zswitch-e2e-home"
SB_MSYS="$UP_MSYS/AppData/Local/Temp/zswitch-e2e-home"
export ZCODE_SWITCH_HOME="$SB_WIN"
PASS=0; FAIL=0

py3() { py -3 "$@"; }

jget() {
  py3 -c "import json,io,sys;d=json.load(io.open(sys.argv[1],encoding='utf-8'));print(eval(sys.argv[2]))" "$(cygpath -w "$1")" "$2"
}

check() {
  if [ "$2" = "$3" ]; then PASS=$((PASS+1)); echo "PASS: $1";
  else FAIL=$((FAIL+1)); echo "FAIL: $1  actual=[$2] expect=[$3]"; fi
}

run() { "$EXE" --cli "$@"; }

rm -rf "$SB_MSYS"; mkdir -p "$SB_MSYS/.zcode/v2"

py3 - <<EOF
import json, io
creds = {
  "oauth:bigmodel:access_token": "enc:v1:E2E_TOKEN_A",
  "oauth:bigmodel:user_info": "enc:v1:E2E_INFO_A",
  "oauth:active_provider": "enc:v1:E2E_ACTIVE",
  "zcodejwttoken": "enc:v1:E2E_JWT_A",
}
io.open(r"$SB_WIN\\.zcode\\v2\\credentials.json", "w", encoding="utf-8").write(json.dumps(creds))
EOF

run state > /tmp/e2e_state.json
check "初始 state ok" "$(jget /tmp/e2e_state.json "d['ok']")" "True"
check "初始已登录" "$(jget /tmp/e2e_state.json "d['live_logged_in']")" "True"
check "初始 0 账号" "$(jget /tmp/e2e_state.json "len(d['accounts'])")" "0"
check "zcode_path 已探测" "$(jget /tmp/e2e_state.json "d['zcode_path_ok']")" "True"

run capture --name 主号A > /tmp/e2e_cap.json
check "capture ok" "$(jget /tmp/e2e_cap.json "d['ok']")" "True"
ID_A=$(jget /tmp/e2e_cap.json "d['id']")

run capture --name 重复 > /tmp/e2e_dup.json
check "重复捕获报错" "$(jget /tmp/e2e_dup.json "d['ok']")" "False"

run rename --id "$ID_A" --name 工作A > /tmp/e2e_ren.json
check "rename ok" "$(jget /tmp/e2e_ren.json "d['name']")" "工作A"
run rename --id "$ID_A" --name "  " > /tmp/e2e_renb.json
check "空名报错" "$(jget /tmp/e2e_renb.json "d['ok']")" "False"

run export --id "$ID_A" --out "$SB_WIN\\exportA.zsb" --password e2e-pass-123 > /tmp/e2e_exp.json
check "export 加密 ok" "$(jget /tmp/e2e_exp.json "d['ok']")" "True"
check "标记 encrypted" "$(jget /tmp/e2e_exp.json "d['encrypted']")" "True"
check "文件是加密信封" "$(jget "$SB_MSYS/exportA.zsb" "d['format']")" "zsw-accounts-bundle"
check "kdf 参数存在" "$(jget "$SB_MSYS/exportA.zsb" "d['kdf']['iters'] == 100000")" "True"
check "密文无明文痕迹" "$(py3 -c "print('oauth' not in open(r'$(cygpath -w "$SB_MSYS/exportA.zsb")', encoding='utf-8').read() and 'E2E_TOKEN' not in open(r'$(cygpath -w "$SB_MSYS/exportA.zsb")', encoding='utf-8').read())")" "True"

py3 - <<EOF
import json, io
creds = {
  "oauth:bigmodel:access_token": "enc:v1:E2E_TOKEN_B",
  "oauth:bigmodel:user_info": "enc:v1:E2E_INFO_B",
  "oauth:active_provider": "enc:v1:E2E_ACTIVE",
  "zcodejwttoken": "enc:v1:E2E_JWT_B",
}
io.open(r"$SB_WIN\\.zcode\\v2\\credentials.json", "w", encoding="utf-8").write(json.dumps(creds))
EOF
run capture --name 二号B > /tmp/e2e_capb.json
ID_B=$(jget /tmp/e2e_capb.json "d['id']")

run switch --id "$ID_B" > /tmp/e2e_sw0.json
check "切到自己是 already_active" "$(jget /tmp/e2e_sw0.json "d['already_active']")" "True"

py3 - <<EOF
import json, io
cfg = {"provider": {"builtin:bigmodel-coding-plan": {"options": {"apiKey": "enc:v1:E2E_CFG_KEY_B", "baseURL": "https://open.bigmodel.cn/api/anthropic"}}}}
io.open(r"$SB_WIN\\.zcode\\v2\\config.json", "w", encoding="utf-8").write(json.dumps(cfg))
creds = {
  "oauth:bigmodel:access_token": "enc:v1:E2E_TOKEN_B_REFRESHED",
  "oauth:bigmodel:user_info": "enc:v1:E2E_INFO_B",
  "oauth:active_provider": "enc:v1:E2E_ACTIVE",
  "zcodejwttoken": "enc:v1:E2E_JWT_B",
}
io.open(r"$SB_WIN\\.zcode\\v2\\credentials.json", "w", encoding="utf-8").write(json.dumps(creds))
EOF
run switch --id "$ID_A" > /tmp/e2e_sw1.json
check "切换成功" "$(jget /tmp/e2e_sw1.json "d['switched']")" "True"
check "漂移登录被保全" "$(jget /tmp/e2e_sw1.json "d['preserved_as'] is not None")" "True"
check "live 已变 A" "$(jget "$SB_MSYS/.zcode/v2/credentials.json" "d['oauth:bigmodel:access_token']")" "enc:v1:E2E_TOKEN_A"
check "config 沿用现网（A 无快照）" "$(jget "$SB_MSYS/.zcode/v2/config.json" "d['provider']['builtin:bigmodel-coding-plan']['options']['apiKey']")" "enc:v1:E2E_CFG_KEY_B"

run state > /tmp/e2e_state2.json
check "active=A" "$(jget /tmp/e2e_state2.json "d['active_account_id']")" "$ID_A"

run switch --id "$ID_B" > /tmp/e2e_sw2.json
check "切到 B 成功" "$(jget /tmp/e2e_sw2.json "d['switched']")" "True"
check "live 已变 B" "$(jget "$SB_MSYS/.zcode/v2/credentials.json" "d['oauth:bigmodel:access_token']")" "enc:v1:E2E_TOKEN_B"

B_MID=$(py3 -c "import json,io;d=json.load(io.open(r'$SB_WIN\\.zcode-switch\\accounts\\$ID_B.json',encoding='utf-8'));print(d.get('virtual_device_mid') or '')")
[ -n "$B_MID" ] || { echo "FAIL: B 应在切换时生成虚拟 mid"; FAIL=$((FAIL+1)); }
check "切换写入设备身份" "$(jget "$SB_MSYS/.zcode/v2/telemetry-state.json" "d['deviceMid']")" "$B_MID"

ZCODE_SWITCH_HOME="$SB_WIN" run export-all --out "$SB_WIN\\allAB.zsb" --password e2e-pass-123 > /tmp/e2e_expall.json
check "export-all 加密 ok" "$(jget /tmp/e2e_expall.json "d['ok']")" "True"
check "bundle 是加密信封" "$(jget "$SB_MSYS/allAB.zsb" "d['format']")" "zsw-accounts-bundle"

SB2_WIN="$UP_WIN\\AppData\\Local\\Temp\\zswitch-e2e-home2"
SB2_MSYS="$UP_MSYS/AppData/Local/Temp/zswitch-e2e-home2"
rm -rf "$SB2_MSYS"; mkdir -p "$SB2_MSYS/.zcode/v2"
py3 - <<EOF
import json, io
creds = {"oauth:bigmodel:access_token": "enc:v1:E2E_TOKEN_C"}
io.open(r"$SB2_WIN\\.zcode\\v2\\credentials.json", "w", encoding="utf-8").write(json.dumps(creds))
EOF
ZCODE_SWITCH_HOME="$SB2_WIN" run capture --name 沙二C > /dev/null
ZCODE_SWITCH_HOME="$SB2_WIN" run import --file "$SB_WIN\\exportA.zsb" --password e2e-pass-123 > /tmp/e2e_imps.json
check "单账号 .zsb 导入成功" "$(jget /tmp/e2e_imps.json "len(d['added'])")" "1"
ZCODE_SWITCH_HOME="$SB2_WIN" run import --file "$SB_WIN\\allAB.zsb" --password wrong-pass > /tmp/e2e_impw.json
check "错密码被拒" "$(jget /tmp/e2e_impw.json "d['ok']")" "False"
ZCODE_SWITCH_HOME="$SB2_WIN" run import --file "$SB_WIN\\allAB.zsb" --password e2e-pass-123 > /tmp/e2e_imp.json
check "解密导入成功 2 个" "$(jget /tmp/e2e_imp.json "len(d['added'])")" "2"
check "已存在被跳过" "$(jget /tmp/e2e_imp.json "len(d['skipped'])")" "1"
check "导入无错误" "$(jget /tmp/e2e_imp.json "len(d['errors'])")" "0"
ZCODE_SWITCH_HOME="$SB2_WIN" run import --file "$SB_WIN\\allAB.zsb" --password e2e-pass-123 > /tmp/e2e_imp2.json
check "重复导入被跳过" "$(jget /tmp/e2e_imp2.json "len(d['skipped'])")" "3"

py3 - <<EOF
import json, io
raw = {"oauth:bigmodel:access_token": "enc:v1:E2E_PLAIN", "zcodejwttoken": "enc:v1:E2E_JWT"}
io.open(r"$SB2_WIN\\plain.json", "w", encoding="utf-8").write(json.dumps(raw))
EOF
ZCODE_SWITCH_HOME="$SB2_WIN" run import --file "$SB2_WIN\\plain.json" > /tmp/e2e_plain.json
check "明文导入被拒" "$(jget /tmp/e2e_plain.json "d['ok']")" "False"
ZCODE_SWITCH_HOME="$SB2_WIN" run list > /tmp/e2e_list2.json
check "账号数不变（4）" "$(jget /tmp/e2e_list2.json "len(d['accounts'])")" "4"

run list > /tmp/e2e_list3.json
BEFORE=$(jget /tmp/e2e_list3.json "len(d['accounts'])")
run delete --id "$ID_A" > /tmp/e2e_del.json
check "delete ok" "$(jget /tmp/e2e_del.json "d['ok']")" "True"
run list > /tmp/e2e_list4.json
check "数量减一" "$(jget /tmp/e2e_list4.json "len(d['accounts'])")" "$((BEFORE-1))"

py3 - <<EOF
import json, io
creds = {"oauth:bigmodel:access_token": "enc:v1:E2E_TOKEN_B_NEW2",
         "oauth:bigmodel:user_info": "enc:v1:E2E_INFO_B",
         "oauth:active_provider": "enc:v1:E2E_ACTIVE",
         "zcodejwttoken": "enc:v1:E2E_JWT_B"}
io.open(r"$SB_WIN\\.zcode\\v2\\credentials.json", "w", encoding="utf-8").write(json.dumps(creds))
EOF
run update --id "$ID_B" > /tmp/e2e_upd.json
check "update ok" "$(jget /tmp/e2e_upd.json "d['ok']")" "True"
run state > /tmp/e2e_state3.json
check "update 后 active=B" "$(jget /tmp/e2e_state3.json "d['active_account_id']")" "$ID_B"

"$EXE" --cli switch --id 不存在的id > /dev/null 2>&1
check "错误退出码非 0" "$([ $? -ne 0 ] && echo yes)" "yes"
"$EXE" --cli state > /dev/null 2>&1
check "正常退出码 0" "$?" "0"

run quota > /tmp/e2e_q1.json
check "quota 假 token 报错" "$(jget /tmp/e2e_q1.json "d['ok']")" "False"
run quota --id "$ID_B" > /tmp/e2e_q2.json
check "quota --id 假 token 报错" "$(jget /tmp/e2e_q2.json "d['ok']")" "False"

run claim-preview > /tmp/e2e_cp.json
check "claim-preview 结构 ok" "$(jget /tmp/e2e_cp.json "d['ok']")" "True"
check "claim-preview 无可领" "$(jget /tmp/e2e_cp.json "d['anyClaimable']")" "False"
check "claim-preview 逐账号项" "$(jget /tmp/e2e_cp.json "len(d['accounts']) >= 1")" "True"
run claim-preview --id "$ID_B" > /tmp/e2e_cp2.json
check "claim-preview --id 过滤" "$(jget /tmp/e2e_cp2.json "len(d['accounts'])")" "1"

run behavior --launch-after-switch 0 --close-to-tray 0 > /tmp/e2e_b1.json
check "behavior 写入" "$(jget /tmp/e2e_b1.json "d['launch_after_switch']")" "False"
run state > /tmp/e2e_b2.json
check "behavior 生效 launch" "$(jget /tmp/e2e_b2.json "d['launch_after_switch']")" "False"
check "behavior 生效 tray" "$(jget /tmp/e2e_b2.json "d['close_to_tray']")" "False"
run behavior --launch-after-switch 1 --close-to-tray 1 > /dev/null
run state > /tmp/e2e_b3.json
check "behavior 恢复默认" "$(jget /tmp/e2e_b3.json "d['launch_after_switch'] and d['close_to_tray']")" "True"

check "state 含 hot_switch 且默认关" "$(jget /tmp/e2e_b3.json "d.get('hot_switch') is False")" "True"

check "auth_proxy 默认关" "$(jget /tmp/e2e_b3.json "d.get('auth_proxy_on') is False")" "True"
py3 - <<EOF
import json, io, os
sp = r"$SB_WIN\.zcode-switch\settings.json"
s = json.load(io.open(sp, encoding="utf-8")) if os.path.exists(sp) else {}
s["auth_proxy_on"] = True
s["auth_proxy_url"] = "socks5://127.0.0.1:1080"
io.open(sp, "w", encoding="utf-8").write(json.dumps(s))
EOF
run state > /tmp/e2e_bp.json
check "auth_proxy 开 + 地址生效" "$(jget /tmp/e2e_bp.json "d['auth_proxy_on'] is True and d['auth_proxy_url'] == 'socks5://127.0.0.1:1080'")" "True"
py3 - <<EOF
import json, io
sp = r"$SB_WIN\.zcode-switch\settings.json"
s = json.load(io.open(sp, encoding="utf-8"))
s["auth_proxy_on"] = False
io.open(sp, "w", encoding="utf-8").write(json.dumps(s))
EOF
run state > /tmp/e2e_bp2.json
check "auth_proxy 关后地址保留" "$(jget /tmp/e2e_bp2.json "d['auth_proxy_on'] is False and d['auth_proxy_url'] == 'socks5://127.0.0.1:1080'")" "True"

check "state 含 live_identity" "$(jget /tmp/e2e_b3.json "'live_identity' in d")" "True"

py3 - <<EOF
import json, io
creds = {"oauth:bigmodel:access_token": "enc:v1:E2E_TOKEN_C",
         "oauth:bigmodel:user_info": "enc:v1:E2E_INFO_C",
         "oauth:active_provider": "enc:v1:E2E_ACTIVE",
         "zcodejwttoken": "enc:v1:E2E_JWT_C"}
io.open(r"$SB_WIN\\.zcode\\v2\\credentials.json", "w", encoding="utf-8").write(json.dumps(creds))
EOF
run capture --name "三号C" > /tmp/e2e_cap3.json
check "capture ok" "$(jget /tmp/e2e_cap3.json "d['ok']")" "True"
ID_C=$(run list | py3 -c "import json,sys;d=json.load(sys.stdin);print([a['id'] for a in d['accounts'] if a['name']=='三号C'][0])")
check "capture 生成虚拟设备" "$(py3 -c "import json,io,io,re;d=json.load(io.open(r'$SB_WIN\\.zcode-switch\\accounts\\$ID_C.json',encoding='utf-8'));print(bool(re.fullmatch(r'[0-9a-f-]{36}', d.get('virtual_device_mid') or '')))")" "True"

echo "=============================="
echo "PASS=$PASS FAIL=$FAIL"
[ $FAIL -eq 0 ] && echo "ALL GREEN" || exit 1
