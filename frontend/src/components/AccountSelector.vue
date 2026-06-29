<template>
  <q-list dense class="gsk-account-list">
    <q-item class="gsk-account-actions" dense>
      <q-btn flat color="primary" to="/accounts/add" size="sm" no-caps class="full-width">
        <q-icon name="add" size="18px" class="q-mr-xs" />添加机器人
      </q-btn>
    </q-item>
    <q-separator class="gsk-sep" />
    <q-item
      clickable
      exact
      v-for="account in accounts"
      :key="account.uin"
      :to="`/accounts/${account.uin}`"
      class="gsk-account-item"
    >
      <q-item-section avatar class="gsk-account-avatar">
        <q-avatar size="32px">
          <q-img :src="`https://q1.qlogo.cn/g?b=qq&nk=${account.uin}&s=640`" />
        </q-avatar>
      </q-item-section>
      <q-item-section>
        <q-item-label class="gsk-account-name">
          {{ account.nickname || account.uin }}
        </q-item-label>
        <q-item-label caption class="gsk-account-meta">
          <span v-if="account.predefined" class="text-orange">配置</span>
          <span v-else class="text-positive">手动</span>
          <span v-if="account.process_running" class="text-positive q-ml-xs">
            <span class="gsk-status-dot online"></span>运行中
          </span>
          <span v-else class="text-negative q-ml-xs">
            <span class="gsk-status-dot offline"></span>已停止
          </span>
        </q-item-label>
      </q-item-section>
    </q-item>

    <q-item v-if="accounts.length === 0 && !loading" class="gsk-empty">
      <q-item-section class="text-center text-muted">
        <q-icon name="sentiment_dissatisfied" size="24px" />
        <div class="text-caption q-mt-xs">暂无机器人</div>
      </q-item-section>
    </q-item>

    <q-inner-loading :showing="loading" />
  </q-list>
</template>
<script setup lang="ts">
import type { AccountListItem } from 'src/api';
import { api } from 'boot/axios';
import { onBeforeUnmount, ref } from 'vue';

const accounts = ref<AccountListItem[]>([]),
  loading = ref(false);

async function getAccounts() {
  try {
    loading.value = true;
    const { data } = await api.allAccountsApiAccountsGet();
    accounts.value = data;
  } finally {
    loading.value = false;
  }
}

void getAccounts();
const updateTimer = window.setInterval(() => void getAccounts(), 5 * 1000);
onBeforeUnmount(() => window.clearInterval(updateTimer));
</script>

<style lang="scss" scoped>
.gsk-account-list {
  padding: 0;
}

.gsk-account-actions {
  padding: 8px 12px !important;
}

.gsk-sep {
  margin: 0 12px;
  background: var(--gsk-border);
}

.gsk-account-item {
  margin: 2px 8px;
  border-radius: 6px;
  transition: all 0.15s ease;

  &:hover {
    background: var(--gsk-surface-hover);
  }

  &.router-link-active,
  &.router-link-exact-active {
    background: rgba(99, 102, 241, 0.1);
  }
}

.gsk-account-avatar {
  min-width: 40px;
  padding-right: 0 !important;
}

.gsk-account-name {
  font-size: 0.85rem;
  font-weight: 500;
}

.gsk-account-meta {
  font-size: 0.7rem;
  display: flex;
  align-items: center;
  gap: 4px;
}

.text-positive { color: var(--gsk-success); }
.text-negative { color: var(--gsk-error); }
.text-muted { color: var(--gsk-text-muted); }

.gsk-empty {
  padding: 16px;
}

.gsk-status-dot {
  display: inline-block;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  &.online { background-color: var(--gsk-success); }
  &.offline { background-color: var(--gsk-text-muted); }
}
</style>
