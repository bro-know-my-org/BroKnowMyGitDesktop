<script lang="ts">
import { computed, defineComponent, onBeforeUnmount, reactive, ref, watch } from "vue";
import { NAlert, NButton, NCard, NCheckbox, NColorPicker, NDescriptions, NDescriptionsItem, NDivider, NDrawer, NDrawerContent, NForm, NFormItem, NInput, NModal, NSelect, NSpace, NSpin, NTabPane, NTabs, NText } from "naive-ui";
import packageMetadata from "../../../../package.json";
import { api } from "../../../shared/api/tauri";
import { diffLineWrapping, setDiffLineWrapping } from "../../../shared/diffPreferences";
import {
  codeFont,
  codeFontSize,
  codeFontWeight,
  codeLigatures,
  interfaceFont,
  interfaceFontSize,
  interfaceFontWeight,
  loadSystemFonts,
  resetFonts,
  setCodeFont,
  setCodeFontSize,
  setCodeFontWeight,
  setCodeLigatures,
  setInterfaceFont,
  setInterfaceFontSize,
  setInterfaceFontWeight,
  systemFontSelection,
  systemCodeFontName,
  systemCodeFontSize,
  systemCodeFontWeight,
  systemFonts,
  systemFontsLoading,
  systemInterfaceFontName,
  systemInterfaceFontSize,
  systemInterfaceFontWeight,
} from "../../../shared/fonts";
import { useWorkspaceControllerContext } from "../composables/workspaceContext";
import type { SystemTerminalInfo, TerminalConfig } from "../../../shared/types/domain";
import { settingsSection } from "../../../shared/settingsNavigation";

export default defineComponent({
  name: "WorkspaceDialogs",
  components: { NAlert, NButton, NCard, NCheckbox, NColorPicker, NDescriptions, NDescriptionsItem, NDivider, NDrawer, NDrawerContent, NForm, NFormItem, NInput, NModal, NSelect, NSpace, NSpin, NTabPane, NTabs, NText },
  setup() {
    const workspace = useWorkspaceControllerContext();
    const narrowSettingsViewport = typeof window === "undefined" || typeof window.matchMedia !== "function"
      ? null
      : window.matchMedia("(max-width: 600px)");
    const settingsTabsPlacement = ref<"left" | "top">(narrowSettingsViewport?.matches ? "top" : "left");
    function updateSettingsTabsPlacement(event: MediaQueryListEvent): void {
      settingsTabsPlacement.value = event.matches ? "top" : "left";
    }
    narrowSettingsViewport?.addEventListener("change", updateSettingsTabsPlacement);
    onBeforeUnmount(() => {
      narrowSettingsViewport?.removeEventListener("change", updateSettingsTabsPlacement);
      systemTerminalRequestId += 1;
      preferredTerminalRequestId += 1;
    });
    const systemTerminal = ref<SystemTerminalInfo | null>(null);
    const preferredTerminal = ref<TerminalConfig | null>(null);
    const systemTerminalLoading = ref(false);
    const systemTerminalLoadFailed = ref(false);
    const preferredTerminalLoading = ref(false);
    const preferredTerminalLoaded = ref(false);
    const preferredTerminalLoadFailed = ref(false);
    const preferredTerminalForm = reactive({ executable: "", arguments: "[]" });
    const preferredTerminalFormDirty = ref(false);
    const systemTerminalDescription = computed(() => {
      if (!systemTerminal.value) return workspace.t("workspace.dialogs.noSystemTerminalCandidateResolved");
      const argumentsDescription = systemTerminal.value.arguments.length
        ? ` ${JSON.stringify(systemTerminal.value.arguments)}`
        : "";
      return `${systemTerminal.value.executable}${argumentsDescription}`;
    });
    const terminalSettingsLoadDeferred = ref(false);
    let terminalSettingsLoadedForCurrentOpen = false;
    let systemTerminalRequestId = 0;
    let preferredTerminalRequestId = 0;

    async function loadSystemTerminalSettings(): Promise<void> {
      if (workspace.mutationBusy.value) {
        terminalSettingsLoadDeferred.value = true;
        return;
      }
      const requestId = ++systemTerminalRequestId;
      systemTerminalLoading.value = true;
      systemTerminalLoadFailed.value = false;
      try {
        const nextSystemTerminal = await api.getSystemTerminal();
        if (requestId !== systemTerminalRequestId) return;
        systemTerminal.value = nextSystemTerminal;
        systemTerminalLoadFailed.value = false;
      } catch (error) {
        if (requestId !== systemTerminalRequestId) return;
        systemTerminal.value = null;
        systemTerminalLoadFailed.value = true;
        workspace.reportError(error, workspace.t("workspace.controller.couldNotLoadSystemTerminal"));
      } finally {
        if (requestId === systemTerminalRequestId) systemTerminalLoading.value = false;
      }
    }

    async function loadPreferredTerminalSettings(): Promise<void> {
      if (workspace.mutationBusy.value) {
        terminalSettingsLoadDeferred.value = true;
        return;
      }
      const requestId = ++preferredTerminalRequestId;
      preferredTerminalLoading.value = true;
      preferredTerminalLoaded.value = false;
      preferredTerminalLoadFailed.value = false;
      try {
        const nextPreferredTerminal = await api.getPreferredTerminal();
        if (requestId !== preferredTerminalRequestId) return;
        preferredTerminalLoaded.value = true;
        preferredTerminalLoadFailed.value = false;
        preferredTerminal.value = nextPreferredTerminal;
        if (!preferredTerminalFormDirty.value) {
          preferredTerminalForm.executable = nextPreferredTerminal?.executable ?? "";
          preferredTerminalForm.arguments = JSON.stringify(nextPreferredTerminal?.arguments ?? [], null, 2);
        }
      } catch (error) {
        if (requestId !== preferredTerminalRequestId) return;
        preferredTerminalLoadFailed.value = true;
        workspace.reportError(error, workspace.t("workspace.controller.couldNotLoadPreferredTerminal"));
      } finally {
        if (requestId === preferredTerminalRequestId) preferredTerminalLoading.value = false;
      }
    }

    function loadTerminalSettings(): void {
      if (workspace.mutationBusy.value) {
        terminalSettingsLoadDeferred.value = true;
        return;
      }
      terminalSettingsLoadDeferred.value = false;
      void loadSystemTerminalSettings();
      void loadPreferredTerminalSettings();
    }

    async function savePreferredTerminal(): Promise<void> {
      if (!preferredTerminalLoaded.value) return;
      if (workspace.mutationBusy.value) {
        workspace.message.warning(workspace.t("workspace.controller.waitForTheCurrentSettingsChangeToFinish"));
        return;
      }
      const executable = preferredTerminalForm.executable;
      if (!executable.trim()) {
        workspace.message.warning(workspace.t("workspace.controller.enterThePreferredTerminalSAbsoluteExecutablePath"));
        return;
      }
      let arguments_: string[];
      try {
        const parsedArguments: unknown = JSON.parse(preferredTerminalForm.arguments);
        if (!Array.isArray(parsedArguments) || !parsedArguments.every((argument) => typeof argument === "string")) {
          throw new TypeError("Preferred terminal arguments must be a string array.");
        }
        arguments_ = parsedArguments;
      } catch {
        workspace.message.warning(workspace.t("workspace.controller.enterPreferredTerminalArgumentsAsAJsonStringArray"));
        return;
      }
      preferredTerminalRequestId += 1;
      preferredTerminalLoading.value = false;
      workspace.mutationBusy.value = true;
      try {
        preferredTerminal.value = await api.savePreferredTerminal({ executable, arguments: arguments_ });
        preferredTerminalForm.executable = preferredTerminal.value?.executable ?? executable;
        preferredTerminalForm.arguments = JSON.stringify(preferredTerminal.value?.arguments ?? [], null, 2);
        preferredTerminalFormDirty.value = false;
        workspace.message.success(workspace.t("workspace.controller.preferredTerminalSaved"));
      } catch (error) {
        workspace.reportError(error, workspace.t("workspace.controller.couldNotSavePreferredTerminal"));
      } finally {
        workspace.mutationBusy.value = false;
      }
    }

    function confirmClearPreferredTerminal(): void {
      workspace.dialog.warning({
        title: workspace.t("workspace.controller.clearPreferredTerminal"),
        content: workspace.t("workspace.controller.clearThePreferredTerminalConfigurationSavedByThisAppThisDoesNotChangeTheSystemDefaultTerminal"),
        positiveText: workspace.t("workspace.controller.clear"),
        negativeText: workspace.t("workspace.controller.cancel"),
        onPositiveClick: async () => {
          if (workspace.mutationBusy.value) {
            workspace.message.warning(workspace.t("workspace.controller.waitForTheCurrentSettingsChangeToFinish"));
            return;
          }
          preferredTerminalRequestId += 1;
          preferredTerminalLoading.value = false;
          workspace.mutationBusy.value = true;
          try {
            preferredTerminal.value = await api.savePreferredTerminal(null);
            preferredTerminalForm.executable = "";
            preferredTerminalForm.arguments = "[]";
            preferredTerminalFormDirty.value = false;
            workspace.message.success(workspace.t("workspace.controller.preferredTerminalConfigurationCleared"));
          } catch (error) {
            workspace.reportError(error, workspace.t("workspace.controller.couldNotClearPreferredTerminal"));
          } finally {
            workspace.mutationBusy.value = false;
          }
        },
      });
    }
    watch(workspace.settingsModalShown, (shown) => {
      if (shown) void loadSystemFonts();
    });
    watch([workspace.settingsModalShown, workspace.mutationBusy, settingsSection], ([shown, busy, section], previous) => {
      const [wasShown, , previousSection] = previous ?? [false, false, "general"];
      const terminalSettingsActive = shown && section === "general";
      const terminalSettingsWereActive = wasShown && previousSection === "general";
      if (!terminalSettingsActive) {
        terminalSettingsLoadDeferred.value = false;
        if (!shown && wasShown) preferredTerminalFormDirty.value = false;
        if (terminalSettingsWereActive || (!shown && wasShown)) {
          const loadWasInFlight = systemTerminalLoading.value || preferredTerminalLoading.value;
          systemTerminalRequestId += 1;
          preferredTerminalRequestId += 1;
          systemTerminalLoading.value = false;
          preferredTerminalLoading.value = false;
          if (!shown || loadWasInFlight) terminalSettingsLoadedForCurrentOpen = false;
        }
      } else if (!terminalSettingsLoadedForCurrentOpen && busy) {
        terminalSettingsLoadDeferred.value = true;
      } else if ((!terminalSettingsLoadedForCurrentOpen || terminalSettingsLoadDeferred.value) && !busy) {
        terminalSettingsLoadedForCurrentOpen = true;
        loadTerminalSettings();
      }
    });
    const interfaceSystemFontOption = computed(() => ({
      label: systemInterfaceFontName.value
        ? workspace.t("workspace.dialogs.systemDefaultFont", { font: systemInterfaceFontName.value })
        : workspace.t("workspace.dialogs.systemDefault"),
      value: systemFontSelection,
    }));
    const codeSystemFontOption = computed(() => ({
      label: systemCodeFontName.value
        ? workspace.t("workspace.dialogs.systemDefaultFont", { font: systemCodeFontName.value })
        : workspace.t("workspace.dialogs.systemDefault"),
      value: systemFontSelection,
    }));
    const interfaceFontOptions = computed(() => [
      interfaceSystemFontOption.value,
      ...systemFonts.value.map((font) => ({ label: font.family, value: font.family })),
    ]);
    const codeFontOptions = computed(() => [
      codeSystemFontOption.value,
      ...systemFonts.value
        .filter((font) => font.monospaced)
        .map((font) => ({ label: font.family, value: font.family })),
    ]);
    function sizeOptions(systemValue: number | null, minimum: number, maximum: number) {
      const systemLabel = systemValue
        ? workspace.t("workspace.dialogs.systemDefaultValue", { value: `${systemValue.toFixed(1)} px` })
        : workspace.t("workspace.dialogs.systemDefault");
      return [
        { label: systemLabel, value: systemFontSelection },
        ...Array.from({ length: maximum - minimum + 1 }, (_, index) => {
          const value = String(minimum + index);
          return { label: `${value} px`, value };
        }),
      ];
    }
    const weightLabels = new Map([
      [100, "Thin"], [200, "ExtraLight"], [300, "Light"], [400, "Regular"], [500, "Medium"],
      [600, "SemiBold"], [700, "Bold"], [800, "ExtraBold"], [900, "Black"],
    ]);
    function weightOptions(systemValue: number | null) {
      const systemLabel = systemValue
        ? workspace.t("workspace.dialogs.systemDefaultValue", { value: `${systemValue} · ${weightLabels.get(systemValue) ?? "Custom"}` })
        : workspace.t("workspace.dialogs.systemDefault");
      return [
        { label: systemLabel, value: systemFontSelection },
        ...Array.from(weightLabels, ([weight, label]) => ({ label: `${weight} · ${label}`, value: String(weight) })),
      ];
    }
    const interfaceFontSizeOptions = computed(() => sizeOptions(systemInterfaceFontSize.value, 10, 24));
    const codeFontSizeOptions = computed(() => sizeOptions(systemCodeFontSize.value, 9, 24));
    const interfaceFontWeightOptions = computed(() => weightOptions(systemInterfaceFontWeight.value));
    const codeFontWeightOptions = computed(() => weightOptions(systemCodeFontWeight.value));
    return {
      ...workspace,
      appVersion: packageMetadata.version === "0.0.0" ? "dev" : packageMetadata.version,
      interfaceFont,
      codeFont,
      interfaceFontSize,
      codeFontSize,
      interfaceFontWeight,
      codeFontWeight,
      codeLigatures,
      diffLineWrapping,
      systemFontsLoading,
      interfaceFontOptions,
      codeFontOptions,
      interfaceFontSizeOptions,
      codeFontSizeOptions,
      interfaceFontWeightOptions,
      codeFontWeightOptions,
      setInterfaceFont,
      setCodeFont,
      setInterfaceFontSize,
      setCodeFontSize,
      setInterfaceFontWeight,
      setCodeFontWeight,
      setCodeLigatures,
      setDiffLineWrapping,
      resetFonts,
      systemTerminal,
      systemTerminalDescription,
      preferredTerminal,
      systemTerminalLoading,
      systemTerminalLoadFailed,
      preferredTerminalLoading,
      preferredTerminalLoaded,
      preferredTerminalLoadFailed,
      preferredTerminalForm,
      preferredTerminalFormDirty,
      loadTerminalSettings,
      loadSystemTerminalSettings,
      loadPreferredTerminalSettings,
      savePreferredTerminal,
      confirmClearPreferredTerminal,
      settingsSection,
      settingsTabsPlacement,
    };
  },
});
</script>

<template>
  <n-modal v-model:show="settingsModalShown" preset="card" :title="t('workspace.dialogs.settings')" class="settings-dialog-card">
    <n-tabs v-model:value="settingsSection" type="line" :placement="settingsTabsPlacement" class="settings-tabs">
      <n-tab-pane name="general" :tab="t('workspace.dialogs.general')">
        <section class="settings-section">
          <div class="settings-section-heading">
            <strong>{{ t('workspace.dialogs.appearanceAndLanguage') }}</strong>
            <n-text depth="3">{{ t('workspace.dialogs.theseOptionsAreSavedAutomaticallyOnThisDevice') }}</n-text>
          </div>
          <n-form label-placement="top" class="settings-form">
            <n-form-item :label="t('workspace.dialogs.language')">
              <n-select :value="appLocale" :options="localeOptions" @update:value="updateAppLocale" />
            </n-form-item>
            <n-form-item :label="t('workspace.dialogs.appearance')">
              <n-select
                :value="colorMode"
                :options="colorModeOptions.map((option) => ({ label: option.label, value: option.key }))"
                @update:value="updateColorMode"
              />
            </n-form-item>
            <n-form-item :label="t('workspace.dialogs.themeColor')">
              <n-space vertical class="theme-color-setting">
                <n-color-picker
                  :value="themeColor"
                  :modes="['hex']"
                  show-alpha
                  :swatches="['#4B0082FF', '#6D28D9FF', '#2563EBFF', '#C026D3FF', '#DC2626FF', '#EA580CFF']"
                  @update:value="updateThemeColor"
                />
                <div class="theme-color-description">
                  <n-text depth="3" class="theme-color-hint">
                    {{ t('workspace.dialogs.alphaControlsTheStrengthOfSelectedBackgroundsBordersAndFocusRings') }}
                  </n-text>
                  <n-button size="small" quaternary @click="resetThemeColor">{{ t('workspace.dialogs.reset') }}</n-button>
                </div>
              </n-space>
            </n-form-item>
            <n-divider />
            <n-form-item :label="t('workspace.dialogs.interfaceFont')">
              <n-select
                filterable
                :loading="systemFontsLoading"
                :value="interfaceFont"
                :options="interfaceFontOptions"
                @update:value="setInterfaceFont"
              />
            </n-form-item>
            <div class="font-setting-grid">
              <n-form-item :label="t('workspace.dialogs.interfaceFontSize')">
                <n-select :value="interfaceFontSize" :options="interfaceFontSizeOptions" @update:value="setInterfaceFontSize" />
              </n-form-item>
              <n-form-item :label="t('workspace.dialogs.interfaceFontWeight')">
                <n-select :value="interfaceFontWeight" :options="interfaceFontWeightOptions" @update:value="setInterfaceFontWeight" />
              </n-form-item>
            </div>
            <n-form-item :label="t('workspace.dialogs.codeFont')">
              <n-select
                filterable
                :loading="systemFontsLoading"
                :value="codeFont"
                :options="codeFontOptions"
                @update:value="setCodeFont"
              />
            </n-form-item>
            <div class="font-setting-grid">
              <n-form-item :label="t('workspace.dialogs.codeFontSize')">
                <n-select :value="codeFontSize" :options="codeFontSizeOptions" @update:value="setCodeFontSize" />
              </n-form-item>
              <n-form-item :label="t('workspace.dialogs.codeFontWeight')">
                <n-select :value="codeFontWeight" :options="codeFontWeightOptions" @update:value="setCodeFontWeight" />
              </n-form-item>
            </div>
            <n-form-item :label="t('workspace.dialogs.programmingLigatures')">
              <n-space vertical class="font-setting">
                <n-checkbox :checked="codeLigatures" @update:checked="setCodeLigatures">
                  {{ t('workspace.dialogs.enableProgrammingLigatures') }}
                </n-checkbox>
                <n-text depth="3" class="font-setting-hint">
                  {{ t('workspace.dialogs.programmingLigaturesRequireAFontThatSupportsThem') }}
                </n-text>
              </n-space>
            </n-form-item>
            <n-button size="small" quaternary @click="resetFonts">{{ t('workspace.dialogs.resetFonts') }}</n-button>
            <n-divider />
            <n-form-item :label="t('workspace.dialogs.diffDisplay')">
              <n-space vertical class="font-setting">
                <n-checkbox :checked="diffLineWrapping" @update:checked="setDiffLineWrapping">
                  {{ t('workspace.dialogs.wrapLongDiffLines') }}
                </n-checkbox>
                <n-text depth="3" class="font-setting-hint">
                  {{ t('workspace.dialogs.wrapLongDiffLinesHint') }}
                </n-text>
              </n-space>
            </n-form-item>
            <n-divider />
            <div class="settings-subsection-heading">
              <strong>{{ t('workspace.dialogs.terminals') }}</strong>
              <n-text depth="3">{{ t('workspace.dialogs.systemDefaultAndPreferredTerminalsAreSeparateTheRepositoryMenuProvidesBothActions') }}</n-text>
            </div>
            <n-form-item :label="t('workspace.dialogs.systemDefaultTerminal')">
              <n-space vertical>
                <n-spin :show="systemTerminalLoading" size="small">
                  <n-text>{{ systemTerminalDescription }}</n-text>
                </n-spin>
                <n-text depth="3">{{ t('workspace.dialogs.systemTerminalCandidateFallbackHint') }}</n-text>
              </n-space>
            </n-form-item>
            <n-alert v-if="systemTerminalLoadFailed" type="warning" :bordered="false">
              <n-space justify="space-between" align="center">
                <span>{{ t('workspace.dialogs.systemDefaultTerminalCouldNotBeDetected') }}</span>
                <n-button size="small" tertiary :loading="systemTerminalLoading" :disabled="mutationBusy" @click="loadSystemTerminalSettings">
                  {{ t('workspace.dialogs.retry') }}
                </n-button>
              </n-space>
            </n-alert>
            <n-form-item :label="t('workspace.dialogs.preferredTerminalExecutable')">
              <n-input
                v-model:value="preferredTerminalForm.executable"
                :disabled="preferredTerminalLoading || mutationBusy || !preferredTerminalLoaded"
                :placeholder="t('workspace.dialogs.forExampleUsrBinKitty')"
                @update:value="preferredTerminalFormDirty = true"
              />
            </n-form-item>
            <n-form-item :label="t('workspace.dialogs.preferredTerminalArgumentsJsonArray')">
              <n-input
                v-model:value="preferredTerminalForm.arguments"
                :disabled="preferredTerminalLoading || mutationBusy || !preferredTerminalLoaded"
                type="textarea"
                :autosize="{ minRows: 2, maxRows: 5 }"
                :placeholder="t('workspace.dialogs.placeholderPreferredTerminalArgumentsJson')"
                @update:value="preferredTerminalFormDirty = true"
              />
            </n-form-item>
            <n-text depth="3" class="terminal-setting-hint">
              {{ t('workspace.dialogs.preferredTerminalUsesAnAbsoluteExecutablePathAndDirectArgumentsNoShellIsUsedRepositoryPlaceholder') }}
            </n-text>
            <n-alert v-if="preferredTerminalLoadFailed" type="warning" :bordered="false">
              <n-space justify="space-between" align="center">
                <span>{{ t('workspace.dialogs.preferredTerminalCouldNotBeLoadedRetryBeforeEditing') }}</span>
                <n-button size="small" tertiary :loading="preferredTerminalLoading" :disabled="mutationBusy" @click="loadPreferredTerminalSettings">
                  {{ t('workspace.dialogs.retry') }}
                </n-button>
              </n-space>
            </n-alert>
            <n-space justify="space-between" class="terminal-setting-actions">
              <n-button
                v-if="preferredTerminal"
                type="error"
                tertiary
                :loading="mutationBusy"
                :disabled="preferredTerminalLoading || mutationBusy || !preferredTerminalLoaded"
                @click="confirmClearPreferredTerminal"
              >
                {{ t('workspace.dialogs.clearConfiguration') }}
              </n-button>
              <span v-else />
              <n-button
                type="primary"
                :loading="mutationBusy"
                :disabled="preferredTerminalLoading || mutationBusy || !preferredTerminalLoaded"
                @click="savePreferredTerminal"
              >
                {{ t('workspace.dialogs.savePreferredTerminal') }}
              </n-button>
            </n-space>
          </n-form>
        </section>
      </n-tab-pane>
      <n-tab-pane name="about" :tab="t('workspace.dialogs.about')">
        <section class="settings-about">
          <div class="about-brand-mark" aria-hidden="true">BK</div>
          <div class="settings-about-heading">
            <strong>BroKnowMyGitDesktop</strong>
            <n-text depth="3">{{ t('workspace.dialogs.versionVersion', { version: appVersion }) }}</n-text>
          </div>
          <p>{{ t('workspace.dialogs.aLightweightMultiAccountDesktopClientForGitAndCodeHostingItKeepsLocalGitAtTheCenterWithIntegrati') }}</p>
          <div class="settings-about-details">
            <span>{{ t('workspace.dialogs.builtWith') }}</span>
            <strong>Tauri 2 · Vue 3 · TypeScript · Rust</strong>
          </div>
        </section>
      </n-tab-pane>
    </n-tabs>
    <template #action>
      <n-space justify="end">
        <n-button type="primary" @click="settingsModalShown = false">{{ t('workspace.dialogs.done') }}</n-button>
      </n-space>
    </template>
  </n-modal>

  <n-modal v-model:show="repositoryAliasModalShown" preset="card" :title="t('workspace.dialogs.setRepositoryAlias')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-form label-placement="top">
      <n-form-item :label="t('workspace.dialogs.repositoryAlias')">
        <n-input v-model:value="repositoryAliasForm.name" maxlength="80" show-count :placeholder="t('workspace.dialogs.forExampleClientMainReleaseTools')" @keyup.enter="saveRepositoryAlias" />
      </n-form-item>
    </n-form>
    <n-text depth="3">{{ t('workspace.dialogs.theAliasHasItsOwnMarkerAndIsShownAlongsideTheRealRepositoryNameItIsStoredOnlyByThisAppAndNeverRe') }}</n-text>
    <template #action><n-space justify="end"><n-button @click="repositoryAliasModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button><n-button type="primary" :loading="mutationBusy" @click="saveRepositoryAlias">{{ t('workspace.dialogs.saveAlias') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="repositoryGroupModalShown" preset="card" :title="t('workspace.dialogs.editRepositoryGroup')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-form label-placement="top">
      <n-form-item :label="t('workspace.dialogs.groupName')">
        <n-input v-model:value="repositoryGroupForm.name" maxlength="80" show-count :placeholder="t('workspace.dialogs.forExampleWorkPersonalClientA')" />
      </n-form-item>
    </n-form>
    <n-text depth="3">{{ t('workspace.dialogs.leaveThisBlankAndSaveToMoveItToUngroupedGroupsAreStoredByThisAppAndNeverChangeTheGitRepository') }}</n-text>
    <template #action><n-space justify="end"><n-button @click="repositoryGroupModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button><n-button type="primary" :loading="mutationBusy" @click="saveRepositoryGroup">{{ t('workspace.dialogs.saveGroup') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="externalEditorModalShown" preset="card" :title="t('workspace.dialogs.externalConflictEditor')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="info" :bordered="false" class="token-alert">
      {{ t('workspace.dialogs.whenConflictsOccurTheAppPassesEveryConflictedFileToThisEditorAsASeparateArgumentItNeverRunsAShel') }}
    </n-alert>
    <n-form label-placement="top">
      <n-form-item :label="t('workspace.dialogs.executableAbsolutePath')">
        <n-input v-model:value="externalEditorForm.executable" :placeholder="t('workspace.dialogs.forExampleUsrBinCodeOrApplicationsVisualStudioCodeAppContentsResourcesAppBinCode')" />
      </n-form-item>
      <n-form-item :label="t('workspace.dialogs.additionalArgumentsOnePerLineOptional')">
        <n-input v-model:value="externalEditorForm.arguments" type="textarea" :autosize="{ minRows: 2, maxRows: 6 }" :placeholder="t('workspace.dialogs.placeholderAdditionalArguments')" />
      </n-form-item>
    </n-form>
    <n-text depth="3">{{ t('workspace.dialogs.enterTheInstalledEditorSAbsoluteExecutablePathVsCodeCanUseArgumentWaitAfterSavingClickResolveInE') }}</n-text>
    <template #action>
      <n-space justify="space-between">
        <n-button v-if="externalEditor" type="error" tertiary :loading="mutationBusy" @click="confirmClearExternalEditor">{{ t('workspace.dialogs.clearConfiguration') }}</n-button>
        <span v-else />
        <n-space>
          <n-button @click="externalEditorModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button>
          <n-button type="primary" :loading="mutationBusy" @click="saveExternalEditor">{{ t('workspace.dialogs.saveEditor') }}</n-button>
        </n-space>
      </n-space>
    </template>
  </n-modal>

  <n-modal
    v-model:show="blameModalShown"
    preset="card"
    :title="t('workspace.dialogs.fileBlame')"
    class="dialog-card"
  >
    <n-text depth="3">{{ selectedChange?.path || '—' }}</n-text>
    <div class="dialog-data-content">
      <n-spin :show="blameLoading">
        <pre v-if="blameText" class="diff-output dialog-output">{{ blameText }}</pre>
        <n-text v-else-if="!blameLoading" depth="3">{{ t('workspace.dialogs.thisFileHasNoBlameRecordsToDisplay') }}</n-text>
      </n-spin>
    </div>
    <template #action><n-space justify="end"><n-button @click="blameModalShown = false">{{ t('workspace.dialogs.close') }}</n-button></n-space></template>
  </n-modal>

  <n-modal
    v-model:show="reflogModalShown"
    preset="card"
    :title="t('workspace.dialogs.reflogRecoveryHistory')"
    class="dialog-card"
  >
    <n-alert type="info" :bordered="false" class="token-alert">{{ t('workspace.dialogs.reflogRecordsLocalReferenceMovementsAndHelpsLocateCommitsBeforeResetsRebasesOrAccidentalChangesU') }}</n-alert>
    <div class="dialog-data-content">
      <n-spin :show="reflogLoading">
        <pre v-if="reflogText" class="diff-output dialog-output">{{ reflogText }}</pre>
        <n-text v-else-if="!reflogLoading" depth="3">{{ t('workspace.dialogs.thisRepositoryHasNoReflogRecordsToDisplay') }}</n-text>
      </n-spin>
    </div>
    <template #action><n-space justify="end"><n-button @click="reflogModalShown = false">{{ t('workspace.dialogs.close') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="cloneModalShown" preset="card" :title="t('workspace.dialogs.cloneRepository')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-form label-placement="top">
      <n-form-item :label="t('workspace.dialogs.remoteUrl')"><n-input v-model:value="cloneForm.url" :placeholder="t('workspace.dialogs.httpsGitExampleComOrgRepositoryGitOrGitHostOrgRepositoryGit')" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.destinationFolder')"><n-input v-model:value="cloneForm.destinationPath" :placeholder="t('workspace.dialogs.placeholderCloneDestination')" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.gitServiceEndpointOptionalForPrivateRepositories')"><n-select v-model:value="cloneForm.endpointId" clearable :options="endpoints.map((endpoint) => ({ label: endpoint.displayName, value: endpoint.id }))" :placeholder="t('workspace.dialogs.leaveEmptyToUseNormalSystemGitBehavior')" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.accountUsedForClone')"><n-select v-model:value="cloneForm.accountId" :options="cloneAccountOptions" /></n-form-item>
    </n-form>
      <n-alert type="info" :bordered="false">{{ t('workspace.dialogs.withoutAnAppAccountCloneUsesSystemGitAndKeepsSshLfsHooksAndYourExistingCredentialHelperCompatibl') }}</n-alert>
    <template #action><n-space justify="end"><n-button @click="cloneModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button><n-button type="primary" :loading="mutationBusy" @click="cloneRepository">{{ t('workspace.dialogs.clone') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="initModalShown" preset="card" :title="t('workspace.dialogs.newLocalRepository')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="info" :bordered="false" class="token-alert">
      {{ t('workspace.dialogs.thisInitializesAnEmptyGitRepositoryAtTheSelectedFolderItDoesNotCreateOrModifyARemoteRepository') }}
    </n-alert>
    <n-form label-placement="top">
      <n-form-item :label="t('workspace.dialogs.newRepositoryFolder')"><n-input v-model:value="initForm.destinationPath" :placeholder="t('workspace.dialogs.placeholderNewRepositoryDestination')" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.initialBranchOptional')"><n-input v-model:value="initForm.initialBranch" :placeholder="t('workspace.dialogs.placeholderDefaultBranch')" /></n-form-item>
    </n-form>
    <template #action><n-space justify="end"><n-button @click="initModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button><n-button type="primary" :loading="mutationBusy" @click="initRepository">{{ t('workspace.dialogs.createRepository') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="discoverModalShown" preset="card" :title="t('workspace.dialogs.discoverLocalRepositories')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-form label-placement="top"><n-form-item :label="t('workspace.dialogs.scanRootFolder')"><n-input v-model:value="discoverForm.rootPath" :placeholder="t('workspace.dialogs.placeholderScanRoot')" /></n-form-item></n-form>
    <n-text depth="3">{{ t('workspace.dialogs.onlyLocalDirectoriesAreReadGitcoreReturnsTheScanResults') }}</n-text>
    <template #action><n-space justify="end"><n-button @click="discoverModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button><n-button type="primary" :loading="mutationBusy" @click="discoverRepositories">{{ t('workspace.dialogs.startScan') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="accountModalShown" preset="card" :title="t('workspace.dialogs.addAuthenticationAccount')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert v-if="accountUsesPat" type="warning" :bordered="false" class="token-alert">{{ t('workspace.dialogs.thePatIsWrittenToTheSystemKeychainOnlyThroughThisIpcRequestDoNotPutATokenInTheLoginOrDisplayName') }}</n-alert>
    <n-alert v-else type="info" :bordered="false" class="token-alert">{{ t('workspace.dialogs.theSshAgentKeepsItsSystemConfigurationAndCanBeBoundExplicitlyToARemoteTheSystemGitCredentialHelp') }}</n-alert>
    <n-form label-placement="top">
      <n-form-item :label="t('workspace.dialogs.gitServiceEndpoint')"><n-select v-model:value="accountForm.endpointId" :options="endpoints.map((endpoint) => ({ label: endpoint.displayName, value: endpoint.id }))" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.authenticationMethod')"><n-select v-model:value="accountForm.authMethod" :options="accountMethodOptions" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.login')"><n-input v-model:value="accountForm.login" :placeholder="t('workspace.dialogs.forExampleYourLogin')" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.displayNameOptional')"><n-input v-model:value="accountForm.displayName" :placeholder="t('workspace.dialogs.forExampleWorkAccount')" /></n-form-item>
      <n-form-item v-if="accountUsesPat" :label="t('workspace.dialogs.personalAccessToken')"><n-input v-model:value="accountForm.token" type="password" show-password-on="click" :placeholder="t('workspace.dialogs.writtenOnceOnly')" /></n-form-item>
    </n-form>
    <n-alert v-if="accountUsesPat && !accountFormCapabilities.patKeychain" type="error" :bordered="false">{{ t('workspace.dialogs.theSelectedEndpointDoesNotDeclarePatKeychainSupportSoItCannotBeSaved') }}</n-alert>
    <template #action><n-space justify="end"><n-button @click="accountModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button><n-button type="primary" :loading="mutationBusy" :disabled="accountUsesPat && !accountFormCapabilities.patKeychain" @click="saveAccount">{{ accountUsesPat ? t('workspace.dialogs.saveToSystemKeychain') : t('workspace.dialogs.saveAccountProfile') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="customEndpointModalShown" preset="card" :title="t('workspace.dialogs.addGitServiceEndpoint')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="info" :bordered="false" class="token-alert">{{ t('workspace.dialogs.aGenericGitServiceNeedsNoWebSiteOrApiSshOrAnHttpsPatCanBeExplicitlyBoundToARemoteTheSystemCreden') }}</n-alert>
    <n-form label-placement="top">
      <n-form-item :label="t('workspace.dialogs.stableIdGitHost')"><n-input v-model:value="endpointForm.id" :placeholder="t('workspace.dialogs.placeholderGitHost')" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.serviceType')"><n-select v-model:value="endpointForm.kind" :options="[{ label: t('workspace.dialogs.genericGitServiceNoPlatformApi'), value: 'genericGit' }, { label: 'GitHub', value: 'gitHub' }, { label: 'GitLab', value: 'gitLab' }, { label: 'Gitee', value: 'gitee' }, { label: 'Gitea', value: 'gitea' }, { label: 'Forgejo', value: 'forgejo' }]" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.displayName')"><n-input v-model:value="endpointForm.displayName" :placeholder="t('workspace.dialogs.companyGitService')" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.webUrlOptionalForGenericGit')"><n-input v-model:value="endpointForm.webUrl" :placeholder="t('workspace.dialogs.placeholderGitWebUrl')" /></n-form-item>
      <n-form-item v-if="endpointForm.kind !== 'genericGit'" :label="t('workspace.dialogs.apiUrl')"><n-input v-model:value="endpointForm.apiUrl" :placeholder="t('workspace.dialogs.placeholderGitApiUrl')" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.gitHostPortOptional')"><n-input v-model:value="endpointForm.gitHost" :placeholder="t('workspace.dialogs.placeholderGitHostWithPort')" /></n-form-item>
    </n-form>
    <template #action><n-space justify="end"><n-button @click="customEndpointModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button><n-button type="primary" :loading="mutationBusy" @click="saveCustomEndpoint">{{ t('workspace.dialogs.saveEndpoint') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="branchModalShown" preset="card" :title="branchForm.checkout ? t('workspace.dialogs.newBranchAndSwitch') : t('workspace.dialogs.newBranch')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="info" :bordered="false" class="token-alert">
      {{ branchForm.checkout ? t('workspace.dialogs.newBranchAndSwitchBehavior') : t('workspace.dialogs.newBranchBehavior') }}
    </n-alert>
    <n-form label-placement="top">
      <n-form-item :label="t('workspace.dialogs.branchName')"><n-input v-model:value="branchForm.name" :placeholder="t('workspace.dialogs.placeholderFeatureBranch')" /></n-form-item>
      <n-form-item><n-checkbox v-model:checked="branchForm.checkout">{{ t('workspace.dialogs.switchToThisBranchAfterCreation') }}</n-checkbox></n-form-item>
      <n-form-item v-if="branchForm.checkout && snapshot?.status.entries.length"><n-checkbox v-model:checked="branchForm.stashLocalChanges">{{ t('workspace.dialogs.stashCurrentChangesBeforeSwitchingToNewBranch') }}</n-checkbox></n-form-item>
    </n-form>
    <template #action><n-space justify="end"><n-button @click="branchModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button><n-button type="primary" :loading="mutationBusy" @click="createBranch">{{ branchForm.checkout ? t('workspace.dialogs.createAndSwitch') : t('workspace.dialogs.createBranch') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="forkModalShown" preset="card" :title="t('workspace.dialogs.forkAndPushCurrentBranch')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="info" :bordered="false" class="token-alert">
      {{ t('workspace.dialogs.forkContributionFlowDescription') }}
    </n-alert>
    <n-form label-placement="top">
      <n-form-item :label="t('workspace.dialogs.ghAccountUsedForFork')"><n-select v-model:value="forkForm.accountId" :options="forkAccountOptions" :placeholder="t('workspace.dialogs.selectAVerifiedGhAccount')" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.forkRemoteName')"><n-input v-model:value="forkForm.remoteName" :placeholder="t('workspace.dialogs.placeholderForkRemoteName')" /></n-form-item>
      <n-form-item>
        <n-checkbox v-model:checked="forkForm.createContributionBranch">
          {{ t('workspace.dialogs.createAndSwitchToContributionBranchBeforeForking') }}
        </n-checkbox>
      </n-form-item>
      <n-form-item v-if="forkForm.createContributionBranch" :label="t('workspace.dialogs.contributionBranchName')"><n-input v-model:value="forkForm.branchName" :placeholder="t('workspace.dialogs.placeholderContributionBranch')" /></n-form-item>
      <n-form-item v-if="forkForm.createContributionBranch && snapshot?.status.entries.length">
        <n-checkbox v-model:checked="forkForm.stashLocalChanges">
          {{ t('workspace.dialogs.stashCurrentChangesBeforeSwitchingToContributionBranch') }}
        </n-checkbox>
      </n-form-item>
    </n-form>
    <n-alert v-if="forkContributionBranchRecommended" type="success" :bordered="false">
      {{ t('workspace.dialogs.defaultBranchContributionRecommendation', { branch: snapshot?.status.branch || '' }) }}
    </n-alert>
    <n-alert v-if="!forkAccountOptions.length" type="warning" :bordered="false">
      {{ t('workspace.dialogs.noUsableGhCliAccountIsAvailableImportASignedInGhIdentityFromAccountsAndAuthenticationFirst') }}
    </n-alert>
    <template #action><n-space justify="end"><n-button @click="forkModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button><n-button type="primary" :loading="mutationBusy" :disabled="!forkAccountOptions.length" @click="forkGithubRepository">{{ t('workspace.dialogs.forkAndPush') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="renameBranchModalShown" preset="card" :title="t('workspace.dialogs.renameBranch')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-form label-placement="top">
      <n-form-item :label="t('workspace.dialogs.currentName')"><n-input :value="renameBranchForm.oldName" readonly /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.newName')"><n-input v-model:value="renameBranchForm.newName" :placeholder="t('workspace.dialogs.placeholderFeatureBranch')" /></n-form-item>
    </n-form>
    <template #action><n-space justify="end"><n-button @click="renameBranchModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button><n-button type="primary" :loading="mutationBusy" @click="renameBranch">{{ t('workspace.dialogs.rename') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="stashModalShown" preset="card" :title="t('workspace.dialogs.stashChanges')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="info" :bordered="false" class="token-alert">
      {{ t('workspace.dialogs.stashRemovesCurrentChangesFromTheWorkingTreeYouCanRestoreThemLaterFromHistoryAndStash') }}
    </n-alert>
    <n-form label-placement="top">
      <n-form-item :label="t('workspace.dialogs.messageOptional')"><n-input v-model:value="stashForm.message" :placeholder="t('workspace.dialogs.forExampleWorkInProgressBeforeSwitchingTasks')" /></n-form-item>
      <n-form-item><n-checkbox v-model:checked="stashForm.includeUntracked">{{ t('workspace.dialogs.includeUntrackedFiles') }}</n-checkbox></n-form-item>
    </n-form>
    <template #action><n-space justify="end"><n-button @click="stashModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button><n-button type="primary" :loading="mutationBusy" @click="stashChanges">{{ t('workspace.dialogs.stashChangesAction') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="rebaseModalShown" preset="card" :title="t('workspace.dialogs.rebaseCurrentBranch')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="warning" :bordered="false" class="token-alert">
      {{ t('workspace.dialogs.rebaseRewritesTheCurrentBranchHistoryIfItWasPushedAlreadyConfirmYourTeamWorkflowBeforeForceWithL') }}
    </n-alert>
    <n-form label-placement="top">
      <n-form-item :label="t('workspace.dialogs.baseBranch')"><n-select v-model:value="rebaseForm.base" :options="rebaseBaseOptions" :placeholder="t('workspace.dialogs.selectALocalBranch')" /></n-form-item>
    </n-form>
    <template #action><n-space justify="end"><n-button @click="rebaseModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button><n-button type="primary" :loading="mutationBusy" @click="startRebase">{{ t('workspace.dialogs.startRebase') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="cherryPickModalShown" preset="card" :title="t('workspace.dialogs.cherryPickCommit')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="info" :bordered="false" class="token-alert">
      {{ t('workspace.dialogs.enterACommitShaFromAnotherLocalOrRemoteTrackingBranchIfItConflictsTheAppPreservesTheStateSoYouCa') }}
    </n-alert>
    <n-form label-placement="top"><n-form-item :label="t('workspace.dialogs.commitShaOrRevision')"><n-input v-model:value="cherryPickForm.revision" :placeholder="t('workspace.dialogs.placeholderCommitRevision')" /></n-form-item></n-form>
    <template #action><n-space justify="end"><n-button @click="cherryPickModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button><n-button type="primary" :loading="mutationBusy" @click="cherryPickCommits">{{ t('workspace.dialogs.cherryPick') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="worktreeModalShown" preset="card" :title="t('workspace.dialogs.newWorktree')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="info" :bordered="false" class="token-alert">
      {{ t('workspace.dialogs.worktreesLetYouCheckOutDifferentBranchesOfTheSameRepositoryInSeparateFoldersABranchCanBeCheckedO') }}
    </n-alert>
    <n-form label-placement="top">
      <n-form-item :label="t('workspace.dialogs.newWorktreeFolder')"><n-input v-model:value="worktreeForm.path" :placeholder="t('workspace.dialogs.placeholderWorktreeDestination')" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.checkoutMode')"><n-select v-model:value="worktreeForm.mode" :options="[{ label: t('workspace.dialogs.createANewBranch'), value: 'new' }, { label: t('workspace.dialogs.checkOutAnExistingBranch'), value: 'existing' }]" /></n-form-item>
      <n-form-item v-if="worktreeForm.mode === 'new'" :label="t('workspace.dialogs.newBranchName')"><n-input v-model:value="worktreeForm.branch" :placeholder="t('workspace.dialogs.placeholderFeatureBranch')" /></n-form-item>
      <n-form-item v-else :label="t('workspace.dialogs.existingBranch')"><n-select v-model:value="worktreeForm.branch" :options="worktreeExistingBranchOptions" :placeholder="t('workspace.dialogs.selectABranchNotCheckedOutElsewhere')" /></n-form-item>
    </n-form>
    <template #action><n-space justify="end"><n-button @click="worktreeModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button><n-button type="primary" :loading="mutationBusy" @click="addWorktree">{{ t('workspace.dialogs.createWorktree') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="lfsTrackModalShown" preset="card" :title="t('workspace.dialogs.addGitLfsTrackingRule')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="info" :bordered="false" class="token-alert">{{ t('workspace.dialogs.theRuleIsWrittenToGitattributesReviewItInTheChangesPanelBeforeCommitting') }}</n-alert>
    <n-form label-placement="top"><n-form-item :label="t('workspace.dialogs.filePattern')"><n-input v-model:value="lfsTrackForm.pattern" :placeholder="t('workspace.dialogs.placeholderLfsPattern')" /></n-form-item></n-form>
    <template #action><n-space justify="end"><n-button @click="lfsTrackModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button><n-button type="primary" :loading="mutationBusy" @click="trackLfsPattern">{{ t('workspace.dialogs.addRule') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="pullRequestModalShown" preset="card" :title="t('workspace.dialogs.createPullRequest')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="warning" :bordered="false" class="token-alert">{{ t('workspace.dialogs.thisCreatesAPullRequestOnGithubThroughTheCurrentlyMatchedGhAccountConfirmThatTheCurrentBranchHas') }}</n-alert>
    <n-form label-placement="top">
      <n-form-item :label="t('workspace.dialogs.headBranch')"><n-input :value="pullRequestHead || snapshot?.status.branch || ''" readonly /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.pullRequestBaseBranch')">
        <div class="pull-request-base-field">
          <n-input v-model:value="pullRequestForm.base" :placeholder="t('workspace.dialogs.placeholderDefaultBranch')" />
          <n-space v-if="pullRequestBaseOptions.length" size="small" wrap class="pull-request-base-suggestions">
            <n-button v-for="option in pullRequestBaseOptions" :key="option.value" size="tiny" tertiary @click="pullRequestForm.base = option.value">{{ option.label }}</n-button>
          </n-space>
        </div>
      </n-form-item>
      <n-form-item :label="t('workspace.dialogs.title')"><n-input v-model:value="pullRequestForm.title" :placeholder="t('workspace.dialogs.brieflyDescribeThisChange')" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.descriptionOptional')"><n-input v-model:value="pullRequestForm.body" type="textarea" :autosize="{ minRows: 4, maxRows: 10 }" /></n-form-item>
      <n-form-item><n-checkbox v-model:checked="pullRequestForm.draft">{{ t('workspace.dialogs.createAsADraftPullRequest') }}</n-checkbox></n-form-item>
    </n-form>
    <template #action><n-space justify="end"><n-button @click="pullRequestModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button><n-button type="primary" :loading="mutationBusy" @click="createPullRequest">{{ t('workspace.dialogs.createPullRequest') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="publishModalShown" preset="card" :title="t('workspace.dialogs.publishToGithub')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="warning" :bordered="false" class="token-alert">{{ t('workspace.dialogs.thisPinsTheSelectedGhCliIdentityToCreateTheRepositoryAddOriginAndPushTheCurrentBranchItNeverSwit') }}</n-alert>
    <n-form label-placement="top">
      <n-form-item :label="t('workspace.dialogs.githubEndpoint')"><n-select v-model:value="publishForm.endpointId" :options="publishEndpoints.map((endpoint) => ({ label: endpoint.displayName, value: endpoint.id }))" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.ghAccountUsedToPublish')"><n-select v-model:value="publishForm.accountId" :options="publishAccountOptions" :placeholder="t('workspace.dialogs.selectAVerifiedGhAccount')" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.repositoryOwner')">
        <n-select v-model:value="publishForm.ownerLogin" :options="publishOwnerOptions" :loading="publishOwnersLoading" :placeholder="t('workspace.dialogs.selectAPersonalAccountOrOrganization')" />
      </n-form-item>
      <n-form-item :label="t('workspace.dialogs.repositoryName')"><n-input v-model:value="publishForm.name" :placeholder="t('workspace.dialogs.placeholderRepositoryName')" /></n-form-item>
      <n-form-item><n-checkbox v-model:checked="publishForm.isPrivate">{{ t('workspace.dialogs.createAPrivateRepository') }}</n-checkbox></n-form-item>
    </n-form>
    <n-alert v-if="!publishAccountOptions.length" type="error" :bordered="false">{{ t('workspace.dialogs.noUsableGhCliAccountIsAvailableImportASignedInGhIdentityFromAccountsAndAuthenticationFirst') }}</n-alert>
    <n-alert v-else-if="publishOwnersError" type="warning" :bordered="false">
      <n-space vertical size="small">
        <span>{{ publishOwnersError }} {{ t('workspace.dialogs.reloadTheOwnersSuccessfullyBeforePublishing') }}</span>
        <n-button size="tiny" tertiary :loading="publishOwnersLoading" @click="loadPublishOwners">{{ t('workspace.dialogs.reloadOrganizations') }}</n-button>
      </n-space>
    </n-alert>
    <template #action><n-space justify="end"><n-button @click="publishModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button><n-button type="primary" :loading="mutationBusy" :disabled="!publishAccountOptions.length || !canPublishToSelectedOwner" @click="publishGitHubRepository">{{ t('workspace.dialogs.createAndPublish') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="squashModalShown" preset="card" :title="t('workspace.dialogs.squashRecentCommits')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="warning" :bordered="false" class="token-alert">{{ t('workspace.dialogs.thisRewritesCurrentBranchHistoryAndRequiresACleanWorktreeIfTheBranchWasPushedAlreadyConfirmWithY') }}</n-alert>
    <n-form label-placement="top">
      <n-form-item :label="t('workspace.dialogs.numberOfRecentCommitsToSquash')"><n-input v-model:value="squashForm.count" inputmode="numeric" :placeholder="t('workspace.dialogs.placeholderSquashCount')" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.newCommitMessage')"><n-input v-model:value="squashForm.message" type="textarea" :autosize="{ minRows: 2, maxRows: 5 }" /></n-form-item>
    </n-form>
    <template #action><n-space justify="end"><n-button @click="squashModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button><n-button type="primary" :loading="mutationBusy" @click="squashRecentCommits">{{ t('workspace.dialogs.squashCommits') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="tagModalShown" preset="card" :title="t('workspace.dialogs.newTag')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-form label-placement="top">
      <n-form-item :label="t('workspace.dialogs.tagName')"><n-input v-model:value="tagForm.name" :placeholder="t('workspace.dialogs.placeholderTagName')" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.targetReference')"><n-select v-model:value="tagForm.target" :options="tagTargetOptions" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.annotationOptional')"><n-input v-model:value="tagForm.message" type="textarea" :autosize="{ minRows: 2, maxRows: 4 }" :placeholder="t('workspace.dialogs.leaveEmptyToCreateALightweightTag')" /></n-form-item>
    </n-form>
    <n-alert type="info" :bordered="false">{{ t('workspace.dialogs.tagsAreCreatedLocallyOnlyPushThemWithYourUsualPushWorkflowOrGitCommand') }}</n-alert>
    <template #action><n-space justify="end"><n-button @click="tagModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button><n-button type="primary" :loading="mutationBusy" @click="createTag">{{ t('workspace.dialogs.createTag') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="remoteModalShown" preset="card" :title="editedRemoteName ? t('workspace.dialogs.editRemote') : t('workspace.dialogs.addRemote')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-form label-placement="top">
      <n-form-item :label="t('workspace.dialogs.remoteName')"><n-input v-model:value="remoteForm.name" :disabled="mutationBusy" :placeholder="t('workspace.dialogs.placeholderRemoteName')" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.fetchUrl')"><n-input v-model:value="remoteForm.fetchUrl" :disabled="mutationBusy" :placeholder="t('workspace.dialogs.placeholderRemoteFetchUrl')" /></n-form-item>
      <n-form-item :label="t('workspace.dialogs.pushUrlOptional')"><n-input v-model:value="remoteForm.pushUrl" :disabled="mutationBusy" :placeholder="t('workspace.dialogs.leaveEmptyToKeepTheExistingPushUrl')" /></n-form-item>
    </n-form>
    <n-alert type="info" :bordered="false">{{ t('workspace.dialogs.remoteUrlsCannotContainTokensOrPasswordsUseARemoteAccountBindingForAuthentication') }}</n-alert>
    <template #action>
      <n-space justify="space-between">
        <n-button v-if="editedRemoteName" type="error" tertiary :loading="mutationBusy" :disabled="mutationBusy" @click="confirmRemoveRemote">{{ t('workspace.dialogs.removeRemote') }}</n-button>
        <span v-else />
        <n-space>
          <n-button :disabled="mutationBusy" @click="remoteModalShown = false">{{ t('workspace.dialogs.cancel') }}</n-button>
          <n-button type="primary" :loading="mutationBusy" :disabled="mutationBusy" @click="saveRemote">{{ t('workspace.dialogs.saveRemote') }}</n-button>
        </n-space>
      </n-space>
    </template>
  </n-modal>

  <n-drawer v-model:show="bindingDrawerShown" :width="440" placement="right">
    <n-drawer-content :title="t('workspace.dialogs.remoteAccountBinding')" closable>
      <template v-if="bindingRemote">
        <n-descriptions :column="1" size="small" label-placement="left">
          <n-descriptions-item :label="t('workspace.dialogs.remote')">{{ bindingRemote.name }}</n-descriptions-item>
          <n-descriptions-item :label="t('workspace.dialogs.url')"><span class="drawer-url">{{ bindingRemote.fetchUrl }}</span></n-descriptions-item>
          <n-descriptions-item :label="t('workspace.dialogs.recognizedService')">{{ endpointKindLabel(bindingRemote.codeHostKind) }}</n-descriptions-item>
        </n-descriptions>
          <n-divider />
          <n-form label-placement="top">
            <n-form-item :label="t('workspace.dialogs.gitServiceEndpoint')"><n-select v-model:value="bindingForm.endpointId" :options="bindingEndpointOptions" /></n-form-item>
            <n-form-item :label="t('workspace.dialogs.accountUsedByThisRemote')"><n-select v-model:value="bindingForm.accountId" :options="bindingAccountOptions" /></n-form-item>
          </n-form>
          <n-button text type="primary" @click="openCustomEndpointModal">{{ t('workspace.dialogs.addGitServiceEndpoint') }}</n-button>
        <n-alert type="info" :bordered="false">
          {{ t('workspace.dialogs.autoContinuesOnlyWhenTheBackendCanDetermineExactlyOneValidAccountMultipleCandidatesOrExpiredAuth') }}
        </n-alert>
        <n-card v-if="bindingForm.accountId !== 'auto'" size="small" class="binding-note">
          {{ t('workspace.dialogs.theSelectedAccountOverridesThisEndpointSDefaultCandidateOnlyForThisRepositoryS') }}<strong>{{ bindingRemote.name }}</strong> {{ t('workspace.dialogs.remoteSentenceSuffix') }}
        </n-card>
      </template>
      <template #footer><n-space justify="end"><n-button @click="bindingDrawerShown = false">{{ t('workspace.dialogs.cancel') }}</n-button><n-button type="primary" :loading="mutationBusy" @click="saveBinding">{{ t('workspace.dialogs.saveBinding') }}</n-button></n-space></template>
    </n-drawer-content>
  </n-drawer>
</template>

<style src="./WorkspaceDialogs.css"></style>
