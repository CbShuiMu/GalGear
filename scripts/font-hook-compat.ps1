# 兼容补丁只应用到临时导出的源码；上游更新导致锚点变化时停止发布。
# 运行时职责及来源见 docs/adr/0003-runtime-font-panel.md。
function Set-FontHookSourceText([string]$Stage, [string]$Relative, [string]$Before, [string]$After, [int]$Count = 1) {
    $path = Join-Path $Stage $Relative
    $text = [IO.File]::ReadAllText($path).Replace("`r`n", "`n")
    if (($text.Split($Before).Count - 1) -ne $Count) {
        throw "Compatibility boundary changed: $Relative. Existing components are preserved."
    }
    [IO.File]::WriteAllText($path, $text.Replace($Before, $After))
}

function Add-FontHookCompatibility([string]$Stage, [string]$Root) {
    $native = Join-Path $Root 'native/font-hook/compat'
    $core = 'SimpleFontHook'
    $pfs = "$core/hooks/internal/engines/artemis/artemis_pfs.cppinc"
    # Whole-file replacements need a stricter boundary than individual anchors.
    $replacements = @{
        "$core/dllmain.cpp" = 'aa6922208a76f8365c192072f0686b6941ab8e89'
        $pfs = '747671ad151ca334ed4fc55860ca53330b90063e'
    }
    foreach ($path in $replacements.Keys) {
        $hash = git hash-object --no-filters (Join-Path $Stage $path)
        if ($LASTEXITCODE -or $hash -ne $replacements[$path]) {
            throw "Upstream replacement boundary changed: $path. Review compatibility before publishing."
        }
    }
    Copy-Item -LiteralPath (Join-Path $native 'artemis_pfs.cppinc') -Destination (Join-Path $Stage $pfs)
    Copy-Item -LiteralPath (Join-Path $native 'entry.cpp') -Destination (Join-Path $Stage "$core/dllmain.cpp")
    foreach ($name in @('face', 'threads', 'pfs', 'table_scale', 'krkr_cache')) {
        Copy-Item -LiteralPath (Join-Path $native "$name.h") -Destination (Join-Path $Stage "$core/galgear_$name.h")
    }
    Set-FontHookSourceText $Stage "$core/SimpleFontHook.vcxproj" '<ClCompile Include="ui\font_picker.cpp" />' @'
<ClCompile Include="ui\font_picker.cpp">
      <ExceptionHandling>Sync</ExceptionHandling>
    </ClCompile>
'@
    $hooks = "$core/hooks/font_hooks.cpp"
    Set-FontHookSourceText $Stage $hooks '#include <detours.h>' @'
#include <detours.h>
#include "../galgear_threads.h"
#include "../galgear_pfs.h"
#include "../galgear_table_scale.h"
#include "../galgear_krkr_cache.h"
'@
    $install = "$core/hooks/internal/font_hooks_install.cppinc"
    Set-FontHookSourceText $Stage $install '    Utils::LoadConfig(hModule);' '    // GalGear owns per-game profiles; do not load a shared FontHook.ini.'
    Set-FontHookSourceText $Stage $install '    DetourTransactionCommit();' @'
    GalGearThreadEnlistment enlisted;
    if (!enlisted.Enlist()) { DetourTransactionAbort(); return; }
    if (DetourTransactionCommit() != NO_ERROR) return;
'@
    Set-FontHookSourceText $Stage $install 'bool handledByLegacyArtemis = ArtemisLegacyNotifyConfigChanged(version);' 'bool handledByLegacyArtemis = !ArtemisLooksLikeEngineRoot() && ArtemisLegacyNotifyConfigChanged(version);'
    Set-FontHookSourceText $Stage $install '    UnityMono::NotifyConfigChanged(version);' "    UnityMono::NotifyConfigChanged(version);`n    GalGearKrkr::Refresh(version);"
    $files = "$core/hooks/internal/engines/engine_file_hooks.cppinc"
    Set-FontHookSourceText $Stage $files 'EngineCommon::IsInternalFileQuery()' '(EngineCommon::IsInternalFileQuery() || GalGearPfs::IsReading())' 10
    $table = "$core/hooks/internal/engines/artemis/artemis_table_patch.cppinc"
    Set-FontHookSourceText $Stage $table '    if (Config::ArtemisFontSize > 0) {' @'
    if (Config::EnableFontHook && Config::EnableFontHeightScale) {
        changed = GalGearTable::Scale(text, static_cast<unsigned>(Config::FontHeightScale * 100.0f + 0.5f)) || changed;
    }
    if (Config::ArtemisFontSize > 0) {
'@
    # 显式返回可修改的内核文件清单；其余文件继续按上游 blob 检查。
    return @("$core/dllmain.cpp", $hooks, $install, $files, $table, $pfs)
}
