<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import {
  NButton,
  NConfigProvider,
  NDescriptions,
  NDescriptionsItem,
  NDrawer,
  NDrawerContent,
  NInput,
  NLayout,
  NLayoutContent,
  NLayoutHeader,
  NMessageProvider,
  NSelect,
  NSpin,
  NTag,
  dateZhCN,
  zhCN,
} from "naive-ui";
import MainArea from "./components/MainArea.vue";
import {
  checkCookies,
  onSidecarMessage,
  type CookieStatus,
  type SidecarMessage,
} from "./lib/sidecar";

const sidecarReady = ref(false);
const settingsOpen = ref(false);
const downloadDir = ref(localStorage.getItem("bilio-download-dir") ?? "~/Downloads/Bilio");
const cookiesBrowser = ref<string | null>(
  localStorage.getItem("bilio-cookies-browser") || null,
);

/** Result of the most recent `check_cookies` probe. */
const cookieStatus = ref<CookieStatus | null>(null);
const cookieStatusLoading = ref(false);

const browserOptions = [
  { label: "无（匿名）", value: "" },
  { label: "Safari", value: "safari" },
  { label: "Chrome", value: "chrome" },
  { label: "Firefox", value: "firefox" },
  { label: "Edge", value: "edge" },
  { label: "Brave", value: "brave" },
  { label: "Chromium", value: "chromium" },
];

onMounted(async () => {
  await onSidecarMessage((msg: SidecarMessage) => {
    if (msg.type === "ready") {
      sidecarReady.value = true;
      // Probe initial cookie status if a browser is already configured.
      if (cookiesBrowser.value) probeCookies();
    }
  });
});

async function probeCookies() {
  if (!cookiesBrowser.value) {
    cookieStatus.value = null;
    return;
  }
  cookieStatusLoading.value = true;
  try {
    cookieStatus.value = await checkCookies(cookiesBrowser.value);
  } catch (e: any) {
    cookieStatus.value = {
      ok: false,
      logged_in: false,
      username: null,
      is_vip: false,
      vip_label: null,
      error: typeof e === "string" ? e : e?.message ?? "读取失败",
    };
  } finally {
    cookieStatusLoading.value = false;
  }
}

// Re-probe whenever the user changes the cookies browser.
watch(cookiesBrowser, (val) => {
  if (val && sidecarReady.value) probeCookies();
  else cookieStatus.value = null;
});

function saveDownloadDir(val: string) {
  downloadDir.value = val;
  localStorage.setItem("bilio-download-dir", val);
}

function saveCookiesBrowser(val: string | null) {
  cookiesBrowser.value = val || null;
  if (val) localStorage.setItem("bilio-cookies-browser", val);
  else localStorage.removeItem("bilio-cookies-browser");
}
</script>

<template>
  <n-config-provider :locale="zhCN" :date-locale="dateZhCN">
    <n-message-provider>
      <n-layout style="min-height: 100vh">
        <n-layout-header bordered class="header">
          <div class="brand">
            <h1>Bilio</h1>
            <span class="tagline">基于 yt-dlp 的 B站视频下载 GUI</span>
          </div>
          <div class="header-right">
            <span class="status" :class="sidecarReady ? 'ok' : 'pending'">
              {{ sidecarReady ? "● sidecar ready" : "○ starting…" }}
            </span>
            <n-button quaternary size="tiny" @click="settingsOpen = true">
              ⚙️
            </n-button>
          </div>
        </n-layout-header>
        <n-layout-content class="content">
          <MainArea
            :download-dir="downloadDir"
            :cookies-browser="cookiesBrowser || undefined"
          />
        </n-layout-content>
      </n-layout>

      <n-drawer v-model:show="settingsOpen" :width="380" placement="right">
        <n-drawer-content title="设置" closable>
          <n-descriptions :columns="1" label-placement="top" bordered size="small">
            <n-descriptions-item label="下载目录">
              <n-input
                :value="downloadDir"
                @update:value="saveDownloadDir"
                placeholder="~/Downloads/Bilio"
                size="small"
              />
            </n-descriptions-item>
            <n-descriptions-item label="使用浏览器 Cookies（账号可访问清晰度）">
              <n-select
                :value="cookiesBrowser ?? ''"
                :options="browserOptions"
                size="small"
                @update:value="saveCookiesBrowser"
              />
              <div v-if="cookiesBrowser" class="cookie-status">
                <n-spin v-if="cookieStatusLoading" size="small" />
                <template v-else-if="cookieStatus">
                  <n-tag
                    v-if="cookieStatus.logged_in"
                    :type="cookieStatus.is_vip ? 'success' : 'info'"
                    size="small"
                    round
                  >
                    {{
                      cookieStatus.is_vip
                        ? `已登录 ${cookieStatus.username} · ${cookieStatus.vip_label}`
                        : `已登录 ${cookieStatus.username}`
                    }}
                  </n-tag>
                  <n-tag
                    v-else-if="cookieStatus.ok"
                    type="warning"
                    size="small"
                    round
                  >
                    Cookies 已读取，但未登录
                  </n-tag>
                  <n-tag v-else type="error" size="small" round>
                    {{ cookieStatus.error ?? "读取失败" }}
                  </n-tag>
                  <n-button
                    quaternary
                    size="tiny"
                    style="margin-left: 0.4rem"
                    @click="probeCookies"
                  >
                    重新检测
                  </n-button>
                </template>
              </div>
            </n-descriptions-item>
            <n-descriptions-item label="ffmpeg">
              <n-tag size="small" type="info">imageio-ffmpeg 自动捆绑</n-tag>
            </n-descriptions-item>
          </n-descriptions>
          <p class="settings-note">
            选择浏览器后，Bilio 会读取该浏览器中已登录 B站的 cookies，从而解析/下载
            账号有权限访问的清晰度或仅登录可见的视频。Cookies 仅在本机使用，不上传任何位置。
            Bilio 不支持破解、DRM 解密或绕过付费验证。
          </p>
        </n-drawer-content>
      </n-drawer>
    </n-message-provider>
  </n-config-provider>
</template>

<style scoped>
.header {
  padding: 0.5rem 1.25rem;
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.brand {
  display: flex;
  align-items: baseline;
  gap: 0.75rem;
}
.brand h1 {
  margin: 0;
  font-size: 1.3rem;
  letter-spacing: 0.5px;
}
.tagline {
  font-size: 0.85rem;
  color: var(--n-text-color-3, #888);
}
.header-right {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}
.status {
  font-size: 0.8rem;
  font-variant-numeric: tabular-nums;
}
.status.ok {
  color: #22a06b;
}
.status.pending {
  color: #b07d00;
}
.content {
  padding: 1rem 1.5rem;
  max-width: 960px;
  margin: 0 auto;
  width: 100%;
  box-sizing: border-box;
}
.settings-note {
  font-size: 0.8rem;
  color: #888;
  margin-top: 1rem;
}
.cookie-status {
  margin-top: 0.5rem;
  display: flex;
  align-items: center;
  gap: 0.3rem;
  flex-wrap: wrap;
}
</style>
<style>
:root {
  font-family:
    -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC",
    "Microsoft YaHei", Roboto, sans-serif;
  text-rendering: optimizeLegibility;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}
html,
body,
#app {
  height: 100%;
  margin: 0;
}
</style>
