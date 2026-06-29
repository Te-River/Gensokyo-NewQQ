<template>
  <q-page class="gsk-dashboard">
    <!-- Page Header -->
    <div class="gsk-page-header">
      <div>
        <div class="gsk-page-title">仪表盘</div>
        <div class="gsk-page-subtitle">系统运行状态概览</div>
      </div>
      <q-chip
        :color="$q.dark.isActive ? 'grey-8' : 'grey-3'"
        text-color="primary"
        icon="refresh"
        size="sm"
      >
        {{ updateInterval }}ms 刷新
      </q-chip>
    </div>

    <!-- Stats Grid -->
    <div class="gsk-stats-grid">
      <!-- CPU Card -->
      <q-card class="gsk-stat-card">
        <q-card-section class="gsk-stat-header">
          <q-icon name="developer_board" size="22px" color="primary" />
          <span class="gsk-stat-label">CPU 占用</span>
        </q-card-section>
        <q-card-section class="gsk-stat-body">
          <div class="gsk-stat-value">
            {{ status?.cpu_percent.toFixed(1) }}<span class="gsk-stat-unit">%</span>
          </div>
          <div class="gsk-stat-detail">
            <span>主进程</span>
            <span class="gsk-stat-num">{{ status?.process.cpu_percent.toFixed(1) }}%</span>
          </div>
          <q-linear-progress
            :value="(status?.cpu_percent ?? 0) / 100"
            color="primary"
            size="4px"
            rounded
            class="q-mt-sm"
          />
        </q-card-section>
      </q-card>

      <!-- Memory Card -->
      <q-card class="gsk-stat-card">
        <q-card-section class="gsk-stat-header">
          <q-icon name="memory" size="22px" color="secondary" />
          <span class="gsk-stat-label">内存占用</span>
        </q-card-section>
        <q-card-section class="gsk-stat-body">
          <div class="gsk-stat-value">
            {{ status?.memory.percent }}<span class="gsk-stat-unit">%</span>
          </div>
          <div class="gsk-stat-detail">
            <span>{{ formatBytes(status?.memory.used ?? 0) }} / {{ formatBytes(status?.memory.total ?? 0) }}</span>
          </div>
          <q-linear-progress
            :value="(status?.memory.percent ?? 0) / 100"
            color="secondary"
            size="4px"
            rounded
            class="q-mt-sm"
          />
        </q-card-section>
      </q-card>

      <!-- Disk Card -->
      <q-card class="gsk-stat-card">
        <q-card-section class="gsk-stat-header">
          <q-icon name="storage" size="22px" color="warning" />
          <span class="gsk-stat-label">硬盘占用</span>
        </q-card-section>
        <q-card-section class="gsk-stat-body">
          <div class="gsk-stat-value">
            {{ status?.disk.percent }}<span class="gsk-stat-unit">%</span>
          </div>
          <div class="gsk-stat-detail">
            <span>{{ formatBytes(status?.disk.free ?? 0) }} / {{ formatBytes(status?.disk.total ?? 0) }}</span>
          </div>
          <q-linear-progress
            :value="(status?.disk.percent ?? 0) / 100"
            color="warning"
            size="4px"
            rounded
            class="q-mt-sm"
          />
        </q-card-section>
      </q-card>

      <!-- Process Info Card -->
      <q-card class="gsk-stat-card">
        <q-card-section class="gsk-stat-header">
          <q-icon name="bolt" size="22px" color="positive" />
          <span class="gsk-stat-label">进程状态</span>
        </q-card-section>
        <q-card-section class="gsk-stat-body">
          <div class="gsk-stat-row">
            <span class="text-muted">主进程内存</span>
            <span class="gsk-stat-num">{{ formatBytes(status?.process.memory_used ?? 0) }}</span>
          </div>
          <div class="gsk-stat-row">
            <span class="text-muted">开机时间</span>
            <span class="gsk-stat-num">{{ new Date((status?.boot_time ?? 0) * 1000).toLocaleString() }}</span>
          </div>
        </q-card-section>
      </q-card>
    </div>

    <!-- Chart + Logs Row -->
    <div class="gsk-bottom-row">
      <!-- Chart -->
      <q-card class="gsk-chart-card">
        <q-card-section class="gsk-stat-header">
          <q-icon name="trending_up" size="22px" color="accent" />
          <span class="gsk-stat-label">资源趋势</span>
          <q-space />
          <q-slider
            class="gsk-interval-slider"
            v-model="updateInterval"
            snap
            :min="500"
            :max="10 * 1000"
            :step="100"
            dense
          />
        </q-card-section>
        <q-card-section>
          <vue-apex-charts
            type="area"
            ref="chart"
            height="220"
            :options="chartOptions"
            :series="chartSeries"
          />
        </q-card-section>
      </q-card>

      <!-- Logs -->
      <logs-console
        @reconnect="processLog"
        :logs="logs"
        :connected="!!logConnection"
        height="100%"
        class="gsk-logs-card"
      />
    </div>
  </q-page>
</template>

<script setup lang="ts">
import { api } from 'src/boot/axios';
import type { SystemStatus } from 'src/api';
import { useQuasar } from 'quasar';
import { onBeforeUnmount, onMounted, watch, ref } from 'vue';
import VueApexCharts from 'vue3-apexcharts';
import type { VueApexChartsComponent } from 'vue3-apexcharts';
import LogsConsole from 'src/components/LogsConsole.vue';

const $q = useQuasar();

const status = ref<SystemStatus>(),
  updateInterval = ref<number>(2000),
  logs = ref<string[]>([]),
  logConnection = ref<WebSocket>();

const LEGEND_NAMES = {
    cpuUsed: 'CPU 总计',
    cpuProcess: 'CPU 主进程',
    memoryUsed: '内存总计',
    memoryProcess: '内存主进程',
  } as const,
  chartOptions = {
    chart: {
      toolbar: { show: false },
      fontFamily: 'Inter, sans-serif',
    },
    dataLabels: { enabled: false },
    xaxis: {
      type: 'datetime' as const,
      range: 1.5 * 60 * 1000,
      labels: { datetimeUTC: false },
    },
    yaxis: {
      max: 100,
      min: 0,
      decimalsInFloat: 1,
      labels: { style: { colors: 'var(--gsk-text-muted)', fontSize: '12px' } },
    },
    stroke: { curve: 'smooth' as const, width: 2 },
    grid: { borderColor: 'var(--gsk-border)' },
    legend: {
      position: 'bottom' as const,
      labels: { colors: 'var(--gsk-text-secondary)' },
    },
    colors: ['#6366f1', '#8b5cf6', '#22c55e', '#06b6d4'],
  },
  chartSeries = Object.values(LEGEND_NAMES).map((name) => ({
    name,
    data: [] as { x: number; y: number }[],
  })),
  chart = ref<VueApexChartsComponent>();

function formatBytes(bytes: number, decimals = 2) {
  if (bytes === 0) return '0 Bytes';
  const k = 1024;
  const dm = decimals < 0 ? 0 : decimals;
  const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB', 'PB', 'EB', 'ZB', 'YB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  const val = parseFloat((bytes / Math.pow(k, i)).toFixed(dm));
  return String(val) + ' ' + sizes[i];
}

let updateTimer: number;

async function updateStatus() {
  try {
    $q.loadingBar.start();
    const { data } = await api.systemStatusApiStatusGet();
    status.value = data;
    const nowDate = Date.now();
    void chart.value?.appendData(
      Object.entries({
        [LEGEND_NAMES.cpuUsed]: data.cpu_percent,
        [LEGEND_NAMES.cpuProcess]: data.process.cpu_percent,
        [LEGEND_NAMES.memoryUsed]: data.memory.percent,
        [LEGEND_NAMES.memoryProcess]:
          (data.process.memory_used / data.memory.total) * 100,
      }).map(([name, value]) => ({
        name,
        data: [{ x: nowDate, y: value }],
      }))
    );
  } finally {
    $q.loadingBar.stop();
    updateTimer = window.setTimeout(() => void updateStatus(), updateInterval.value);
  }
}

async function processLog() {
  const { data } = await api.systemLogsHistoryApiLogsGet();
  logs.value = data;
  logConnection.value?.close();
  const wsUrl = new URL('api/logs', location.href);
  wsUrl.protocol = wsUrl.protocol === 'https:' ? 'wss:' : 'ws:';
  logConnection.value = new WebSocket(wsUrl.href);
  logConnection.value.onmessage = ({ data }) => logs.value.push(data as string);
  logConnection.value.onclose = () => (logConnection.value = undefined);
}

onMounted(() => {
  void updateStatus();
  void processLog();
});

onBeforeUnmount(() => {
  window.clearTimeout(updateTimer);
  logConnection.value?.close();
});

watch(
  () => $q.dark.isActive,
  () => {
    const theme = $q.dark.isActive ? 'dark' : 'light';
    void chart.value?.updateOptions({ theme: { mode: theme } });
  },
  { immediate: true }
);
</script>

<style lang="scss" scoped>
.gsk-dashboard {
  padding: 24px;
  max-width: 1600px;
  margin: 0 auto;
}

.gsk-page-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  margin-bottom: 24px;
}

.gsk-page-title {
  font-size: 1.5rem;
  font-weight: 700;
  color: var(--gsk-text);
}

.gsk-page-subtitle {
  font-size: 0.875rem;
  color: var(--gsk-text-muted);
  margin-top: 2px;
}

// Stats Grid
.gsk-stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: 16px;
  margin-bottom: 24px;
}

.gsk-stat-card {
  border: 1px solid var(--gsk-border);
  border-radius: 12px;
  overflow: hidden;
}

.gsk-stat-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 14px 16px 0 !important;
}

.gsk-stat-label {
  font-size: 0.8rem;
  font-weight: 500;
  color: var(--gsk-text-secondary);
}

.gsk-stat-body {
  padding: 12px 16px 16px !important;
}

.gsk-stat-value {
  font-size: 2rem;
  font-weight: 700;
  color: var(--gsk-text);
  line-height: 1.1;
  margin-bottom: 4px;
}

.gsk-stat-unit {
  font-size: 1rem;
  font-weight: 500;
  color: var(--gsk-text-muted);
  margin-left: 2px;
}

.gsk-stat-detail {
  display: flex;
  justify-content: space-between;
  font-size: 0.8rem;
  color: var(--gsk-text-muted);
}

.gsk-stat-num {
  font-weight: 500;
  color: var(--gsk-text-secondary);
}

.gsk-stat-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 4px 0;
}

.text-muted { color: var(--gsk-text-muted); }

// Bottom row
.gsk-bottom-row {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 16px;
}

@media (max-width: 1024px) {
  .gsk-bottom-row {
    grid-template-columns: 1fr;
  }
}

.gsk-chart-card {
  border: 1px solid var(--gsk-border);
  border-radius: 12px;
}

.gsk-interval-slider {
  max-width: 140px;
}

.gsk-logs-card {
  border: 1px solid var(--gsk-border);
  border-radius: 12px;
}
</style>
