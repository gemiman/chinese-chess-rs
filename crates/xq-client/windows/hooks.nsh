; 安装包钩子（由 tauri.conf.json 的 bundle.windows.nsis.installerHooks 引入）。
;
; # 为什么需要它
;
; 实测：**静默卸载**（`uninstall.exe /S`）会把安装目录删干净、卸载器返回 0，
; 但**开始菜单与桌面那两个快捷方式留在原地**。用户从「设置 → 应用」卸载时若也只看到
; 快捷方式还在，就会以为没卸干净。
;
; 这里显式删一次。删不存在的文件在 NSIS 里是空操作，所以就算模板自己已经删过，
; 多删一次也不会有副作用 —— 要的是**两种卸载方式都能干净**，而不是去猜模板怎么实现的。
;
; 快捷方式名与 `${PRODUCTNAME}` 一致（即 tauri.conf.json 的 productName「Yidao」），
; 两者都在安装时由同一份模板创建。

!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$DESKTOP\${PRODUCTNAME}.lnk"
  Delete "$SMPROGRAMS\${PRODUCTNAME}.lnk"
!macroend
