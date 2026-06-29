<template>
  <q-page class="gsk-detail-page">
    <!-- Page Header -->
    <div class="gsk-page-header">
      <div class="gsk-page-header-left">
        <q-avatar size="40px">
          <q-img :src="`https://q1.qlogo.cn/g?b=qq&nk=${uin}&s=640`" />
        </q-avatar>
        <div>
          <div class="gsk-page-title">机器人 #{{ uin }}</div>
          <div class="gsk-page-subtitle">进程监控与管理</div>
        </div>
      </div>
      <div class="gsk-header-actions">
        <q-btn flat color="primary" label="频道/群列表" icon="list" :to="`/list/${uin}`" size="sm" no-caps />
        <q-btn flat color="secondary" label="配置" icon="settings" :to="`/accounts/${uin}/config`" size="sm" no-caps />
        <q-btn flat color="accent" label="设备" icon="smartphone" :to="`/accounts/${uin}/device`" size="sm" no-caps />
      </div>
    </div>

    <div class="gsk-detail-grid">
      <!-- Left Column -->
      <div class="gsk-detail-left">
        <!-- Status Card -->
        <q-card class="gsk-card gsk-detail-status">
          <q-card-section class="gsk-card-header">
            <q-icon name="monitor_heart" size="20px" color="primary" />
            <span>进程状态</span>
          </q-card-section>

          <q-slide-transition>
            <q-card-section v-if="status" class="gsk-status-content">
              <div class="gsk-status-badge" v-if="status.status === 'running'">
                <span class="gsk-status-dot online"></span>
                <span class="text-positive fw-500">运行中</span>
              </div>
              <div class="gsk-status-badge" v-else>
                <span class="gsk-status-dot offline"></span>
                <span class="text-muted">已停止</span>
              </div>

              <running-process-status v-if="status.status === 'running' && status.details" :status="status.details" />
              <div v-else-if="status.details">
                <q-chip size="sm" color="negative" text-color="white" icon="error">
                  退出代码: {{ status.details.code }}
                </q-chip>
              </div>

              <div class="gsk-stat-chips">
                <q-chip size="sm" outline color="primary" icon="description">
                  日志 {{ status.total_logs }} 条
                </q-chip>
                <q-chip size="sm" outline color="warning" icon="restart_alt">
                  重启 {{ status.restarts }} 次
                </q-chip>
              </div>

              <q-slide-transition v-if="status.qr_uri">
                <q-btn push icon="qr_code" color="accent" size="sm" class="q-mt-sm">
                  二维码
                  <q-popup-proxy>
                    <q-img width="200px" :src="status.qr_uri" />
                  </q-popup-proxy>
                </q-btn>
              </q-slide-transition>
            </q-card-section>
          </q-slide-transition>

          <q-card-actions class="gsk-card-actions">
            <q-btn flat color="negative" icon="stop" @click="stopProcess" label="停止" size="sm" no-caps />
            <q-btn flat color="positive" icon="play_arrow" @click="startProcess" label="启动" size="sm" no-caps />
            <q-space />
            <q-btn flat color="grey-6" icon="refresh" @click="updateStatus" size="sm" round />
          </q-card-actions>
        </q-card>

        <!-- Message Sender -->
        <message-sender :uin="uin" class="gsk-card" />
      </div>

      <!-- Right Column - Logs -->
      <logs-console
        @reconnect="processLog"
        :logs="logs"
        :connected="!!logConnection"
        class="gsk-card gsk-detail-logs"
      >
        <template v-slot:top-trailing>
          <q-checkbox
            v-model="enableInput"
            checked-icon="menu_open"
            unchecked-icon="menu"
            color="secondary"
            dense
            size="sm"
          />
        </template>
        <template v-slot:top>
          <q-slide-transition>
            <q-card-section v-show="enableInput" class="q-pb-none">
              <q-input v-model="stdinInput" outlined dense label="传入文字到进程" bg-color="transparent">
                <template v-slot:after>
                  <q-btn icon="input" flat color="accent" round size="sm" @click="sendStdin" />
                </template>
              </q-input>
            </q-card-section>
          </q-slide-transition>
        </template>
      </logs-console>
    </div>
  </q-page>
</template>
<script setup lang="ts">
import { onBeforeUnmount, ref } from 'vue';
import { useQuasar } from 'quasar';
import { api } from 'boot/axios';
import type { ProcessInfo, ProcessLog } from 'src/api';
import RunningProcessStatus from 'components/RunningProcessStatus.vue';
import LogsConsole from 'components/LogsConsole.vue';
import MessageSender from 'src/components/MessageSender.vue';

const $q = useQuasar();

const props = defineProps<{ uin: number }>(),
  status = ref<ProcessInfo>(),
  logs = ref<ProcessLog[]>([]),
  logConnection = ref<WebSocket>(),
  enableInput = ref(false),
  stdinInput = ref('');

async function updateStatus() {
  logConnection.value?.send('heartbeat');
  try {
    $q.loadingBar.start();
    const { data } = await api.processStatusApiUinProcessStatusGet(props.uin);
    status.value = data;
  } catch (err) {
    console.error(err);
  } finally {
    $q.loadingBar.stop();
  }
}

async function stopProcess() {
  try {
    $q.loading.show();
    await api.processStopApiUinProcessDelete(props.uin);
    await updateStatus();
  } catch (err) {
    console.error(err);
  } finally {
    $q.loading.hide();
  }
}

async function startProcess() {
  try {
    $q.loading.show();
    await api.processStartApiUinProcessPut(props.uin);
    await updateStatus();
  } catch (err) {
    console.error(err);
  } finally {
    $q.loading.hide();
  }
}

async function sendStdin() {
  try {
    $q.loading.show();
    await api.processInputLineApiUinProcessLogsPost(props.uin, {
      input: stdinInput.value,
    });
  } catch (err) {
    console.error(err);
  } finally {
    $q.loading.hide();
  }
}

let lastConnectionTime = 0;
const connectionCooldown = 2000; // 1秒间隔

async function processLog() {
  const currentTime = Date.now();

  // 如果当前时间与上次连接时间小于1秒，则不再次连接
  if (currentTime - lastConnectionTime < connectionCooldown) {
    return;
  }

  // 更新上次连接时间
  lastConnectionTime = currentTime;

  // 获取日志数据
  const { data } = await api.processLogsHistoryApiUinProcessLogsGet(props.uin);
  logs.value = data;

  // 如果WebSocket连接已经打开，直接返回不再重新连接
  if (
    logConnection.value &&
    logConnection.value.readyState === WebSocket.OPEN
  ) {
    return;
  }

  // 关闭现有的WebSocket连接
  logConnection.value?.close();

  // 建立新的WebSocket连接
  const wsUrl = new URL(`api/${props.uin}/process/logs`, location.href);
  wsUrl.protocol = wsUrl.protocol.replace('http', 'ws');

  logConnection.value = new WebSocket(wsUrl.href);
  logConnection.value.onmessage = ({ data }) => {
    logs.value.push(JSON.parse(data as string) as ProcessLog);
  };
  logConnection.value.onclose = () => {
    logConnection.value = undefined;
  };
}

import { watch } from 'vue';

const updateTimer = window.setInterval(() => void updateStatus(), 3000);

watch(
  () => props.uin,
  async () => {
    status.value = undefined;
    logs.value = [];
    try {
      $q.loading.show();
      await updateStatus();
      await processLog();
    } finally {
      $q.loading.hide();
    }
  },
  { immediate: true }
);

onBeforeUnmount(() => {
  window.clearInterval(updateTimer);
  logConnection.value?.close();
});

void updateStatus();
</script>

<style lang="scss" scoped>
.gsk-detail-page {
  padding: 24px;
  max-width: 1600px;
  margin: 0 auto;
}

.gsk-page-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 24px;
  flex-wrap: wrap;
  gap: 12px;
}

.gsk-page-header-left {
  display: flex;
  align-items: center;
  gap: 12px;
}

.gsk-page-title {
  font-size: 1.25rem;
  font-weight: 700;
  color: var(--gsk-text);
}

.gsk-page-subtitle {
  font-size: 0.8rem;
  color: var(--gsk-text-muted);
}

.gsk-header-actions {
  display: flex;
  gap: 8px;
}

.gsk-detail-grid {
  display: grid;
  grid-template-columns: 380px 1fr;
  gap: 16px;
}

@media (max-width: 1024px) {
  .gsk-detail-grid {
    grid-template-columns: 1fr;
  }
}

.gsk-detail-left {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.gsk-card {
  border: 1px solid var(--gsk-border);
  border-radius: 12px;
  overflow: hidden;
}

.gsk-card-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 14px 16px !important;
  font-size: 0.9rem;
  font-weight: 600;
  border-bottom: 1px solid var(--gsk-border);
}

.gsk-card-actions {
  padding: 8px 12px !important;
  border-top: 1px solid var(--gsk-border);
}

.gsk-status-content {
  padding: 16px !important;
}

.gsk-status-badge {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
}

.gsk-stat-chips {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
  margin-top: 12px;
}

.text-positive { color: var(--gsk-success); }
.text-muted { color: var(--gsk-text-muted); }
.fw-500 { font-weight: 500; }
</style>