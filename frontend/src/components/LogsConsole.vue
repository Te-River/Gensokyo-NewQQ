<template>
  <div class="gsk-logs-wrapper">
    <div class="gsk-logs-header">
      <q-icon name="terminal" size="18px" color="primary" />
      <span class="gsk-logs-title">进程日志</span>
      <q-space />
      <q-chip
        v-if="typeof connected === 'boolean'"
        @click="(event) => reconnect('reconnect', event)"
        :clickable="!connected"
        :color="connected ? 'positive' : 'negative'"
        :icon="connected ? 'link' : 'link_off'"
        size="sm"
        text-color="white"
      >
        {{ connected ? '实时' : '断开' }}
      </q-chip>
      <q-btn
        @click="scroll?.setScrollPercentage('vertical', 1, 300)"
        flat
        rounded
        icon="move_down"
        size="sm"
      >
        <q-tooltip>跳转底部</q-tooltip>
      </q-btn>
      <slot name="top-trailing" />
    </div>
    <slot name="top" />
    <q-scroll-area
      ref="scroll"
      class="gsk-logs-area"
      :style="{ height: height ?? 'calc(100vh - 12rem)' }"
    >
      <div ref="root" class="gsk-logs-content ansi-up-theme">
        <div class="gsk-log-line" :key="index" v-for="(line, index) in logs">
          <div v-if="typeof line === 'string'">
            <code v-html="converter.ansi_to_html(line)" />
          </div>
          <div v-else :class="{ start: line.message.startsWith(START_LINE_MARK) }">
            <code v-if="line.time" class="gsk-log-time">
              {{ new Date(line.time).toLocaleString() }}
            </code>
            <code v-if="line.level" class="gsk-log-level">{{ line.level }}</code>
            <code
              v-html="converter.ansi_to_html(line.message)"
              :class="LOG_LEVEL_MAP[line.level ?? ProcessLogLevel.Stdout]"
            />
          </div>
        </div>
      </div>
    </q-scroll-area>
  </div>
</template>
<script setup lang="ts">
import { nextTick, watch, ref } from 'vue';
import { QScrollArea } from 'quasar';
import { AnsiUp } from 'ansi_up';

import { type ProcessLog, ProcessLogLevel } from 'src/api';

const START_LINE_MARK = '当前版本:',
  LOG_LEVEL_MAP: Record<string, string> = {
    [ProcessLogLevel.Debug]: 'level-debug',
    [ProcessLogLevel.Info]: 'level-info',
    [ProcessLogLevel.Warning]: 'level-warn',
    [ProcessLogLevel.Error]: 'level-error',
    [ProcessLogLevel.Fatal]: 'level-fatal',
    [ProcessLogLevel.Stdout]: 'stdout',
  };

const converter = new AnsiUp();
converter.use_classes = true;

const emit = defineEmits(['reconnect']);

const props = defineProps<{
    logs: ProcessLog[] | string[];
    connected?: boolean;
    height?: string;
  }>(),
  root = ref<HTMLElement>(),
  scroll = ref<QScrollArea>();

watch(
  () => props.logs.length,
  async () => {
    if (!scroll.value) return;
    const wrapper = scroll.value.getScrollTarget();
    const { scrollTop, clientHeight, scrollHeight } = wrapper;
    if (Math.abs(scrollTop + clientHeight - scrollHeight) <= 1) {
      await nextTick();
      wrapper.scrollTop = scrollHeight;
    }
  }
);
</script>

<style lang="scss">
@import '~@fontsource/roboto-mono/index.css';

.gsk-logs-wrapper {
  border: 1px solid var(--gsk-border);
  border-radius: 12px;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

.gsk-logs-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  border-bottom: 1px solid var(--gsk-border);
  background: var(--gsk-surface);
}

.gsk-logs-title {
  font-size: 0.85rem;
  font-weight: 600;
  color: var(--gsk-text);
}

.gsk-logs-area {
  background: #0d1117;
  color: #c9d1d9;
}

.gsk-logs-content {
  padding: 12px 14px;
  font-family: 'Roboto Mono', monospace;
  font-size: 13px;
  line-height: 1.6;
}

.gsk-log-line {
  padding: 1px 4px;
  border-radius: 2px;
  white-space: pre-wrap;

  &:hover {
    background: rgba(255, 255, 255, 0.03);
  }

  &.start {
    margin-top: 12px;
    border-top: 1px solid rgba(140, 149, 159, 0.4);
    padding-top: 12px;
  }
}

.gsk-log-time {
  color: #8b949e;
  font-size: 11px;
  font-weight: 600;
  margin-right: 6px;
}

.gsk-log-level {
  color: #c9d1d9;
  font-weight: 600;
  margin-right: 6px;
}

// Level colors
.level-debug { color: #58a6ff; }
.level-info { color: #79c0ff; }
.level-warn { color: #d29922; }
.level-error { color: #f85149; }
.level-fatal { color: #ff7b72; font-weight: 700; }
.stdout { color: #c9d1d9; }
</style>
<style lang="scss">
.ansi-up-theme {
  .ansi-black-fg { color: #484f58; }
  .ansi-black-bg { background-color: #484f58; }
  .ansi-black-intense-bg {
    background-color: #282c36;
  }
  .ansi-red-fg {
    color: #e75c58;
  }
  .ansi-red-bg {
    background-color: #e75c58;
  }
  .ansi-red-intense-fg {
    color: #b22b31;
  }
  .ansi-red-intense-bg {
    background-color: #b22b31;
  }
  .ansi-green-fg {
    color: #00a250;
  }
  .ansi-green-bg {
    background-color: #00a250;
  }
  .ansi-green-intense-fg {
    color: #007427;
  }
  .ansi-green-intense-bg {
    background-color: #007427;
  }
  .ansi-yellow-fg {
    color: #ddb62b;
  }
  .ansi-yellow-bg {
    background-color: #ddb62b;
  }
  .ansi-yellow-intense-fg {
    color: #b27d12;
  }
  .ansi-yellow-intense-bg {
    background-color: #b27d12;
  }
  .ansi-blue-fg {
    color: #208ffb;
  }
  .ansi-blue-bg {
    background-color: #208ffb;
  }
  .ansi-blue-intense-fg {
    color: #0065ca;
  }
  .ansi-blue-intense-bg {
    background-color: #0065ca;
  }
  .ansi-magenta-fg {
    color: #d160c4;
  }
  .ansi-magenta-bg {
    background-color: #d160c4;
  }
  .ansi-magenta-intense-fg {
    color: #a03196;
  }
  .ansi-magenta-intense-bg {
    background-color: #a03196;
  }
  .ansi-cyan-fg {
    color: #60c6c8;
  }
  .ansi-cyan-bg {
    background-color: #60c6c8;
  }
  .ansi-cyan-intense-fg {
    color: #258f8f;
  }
  .ansi-cyan-intense-bg {
    background-color: #258f8f;
  }
  .ansi-white-fg {
    color: #c5c1b4;
  }
  .ansi-white-bg {
    background-color: #c5c1b4;
  }
  .ansi-white-intense-fg {
    color: #a1a6b2;
  }
  .ansi-white-intense-bg {
    background-color: #a1a6b2;
  }

  .ansi-default-inverse-fg {
    color: #ffffff;
  }
  .ansi-default-inverse-bg {
    background-color: #000000;
  }

  .ansi-bold {
    font-weight: bold;
  }
  .ansi-underline {
    text-decoration: underline;
  }
}
</style>
