<template>
  <div class="gsk-process-stats">
    <div class="gsk-process-stat">
      <q-icon name="developer_board" size="16px" color="primary" />
      <span class="gsk-stat-label">CPU</span>
      <span class="gsk-stat-val">{{ status?.cpu_percent }}%</span>
    </div>
    <div class="gsk-process-stat">
      <q-icon name="account_tree" size="16px" color="secondary" />
      <span class="gsk-stat-label">PID</span>
      <span class="gsk-stat-val">{{ status?.pid }}</span>
    </div>
    <div class="gsk-process-stat">
      <q-icon name="memory" size="16px" color="warning" />
      <span class="gsk-stat-label">内存</span>
      <span class="gsk-stat-val">{{ ((status?.memory_used ?? 0) / 1024 ** 2).toFixed(1) }}MB</span>
    </div>
    <div class="gsk-process-stat">
      <q-icon name="timer" size="16px" color="positive" />
      <span class="gsk-stat-label">在线</span>
      <span class="gsk-stat-val">{{ formatTimeDelta(uptime) }}</span>
    </div>
  </div>
</template>
<script setup lang="ts">
import type { RunningProcessDetail } from 'src/api';
import { onMounted, onUnmounted, ref } from 'vue';

const uptime = ref(0),
  props = defineProps<{ status: RunningProcessDetail }>();

let uptimeRefreshTimer: number;

function formatTimeDelta(delta: number) {
  const d = Math.floor(delta / 86400000);
  const h = Math.floor((delta % 86400000) / 3600000);
  const m = Math.floor((delta % 3600000) / 60000);
  const s = Math.floor((delta % 60000) / 1000);
  const daysStr = d ? String(d) + '天' : '';
  return `${daysStr}${h}时${m}分${s}秒` || '0秒';
}

onMounted(() => {
  uptimeRefreshTimer = window.setInterval(() => {
    uptime.value = Date.now() - (props.status?.start_time ?? 0) * 1000;
  }, 1000);
});

onUnmounted(() => {
  clearInterval(uptimeRefreshTimer);
});
</script>

<style lang="scss" scoped>
.gsk-process-stats {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px;
}

.gsk-process-stat {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  border-radius: 8px;
  background: var(--gsk-surface-soft);
}

.gsk-stat-label {
  font-size: 0.75rem;
  color: var(--gsk-text-muted);
}

.gsk-stat-val {
  margin-left: auto;
  font-size: 0.8rem;
  font-weight: 600;
  color: var(--gsk-text);
}
</style>
