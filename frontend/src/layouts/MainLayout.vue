<template>
  <q-layout view="lHh Lpr lFf" class="gsk-layout">
    <!-- Mini Sidebar -->
    <q-header class="gsk-header" :class="$q.dark.isActive ? 'text-white' : ''">
      <q-toolbar class="gsk-toolbar">
        <q-btn flat dense round icon="menu" size="sm" @click="toggleLeftDrawer" class="q-mr-sm" />
        <q-btn icon="dashboard" to="/index" flat dense round size="sm" class="q-mr-sm" />
        <q-toolbar-title class="gsk-toolbar-title">
          <span class="gsk-gradient-text fw-600">Gensokyo</span>
          <span class="text-muted q-ml-xs" style="font-size: 0.85rem">控制台</span>
        </q-toolbar-title>
        <q-space />
        <q-btn
          :icon="$q.dark.isActive ? 'dark_mode' : 'light_mode'"
          @click="$q.dark.toggle"
          flat
          dense
          round
          size="sm"
        >
          <q-tooltip>{{ $q.dark.isActive ? '切换亮色' : '切换暗色' }}</q-tooltip>
        </q-btn>
        <q-separator spaced vertical class="q-mx-xs" />
        <q-btn icon="info" flat dense round size="sm">
          <q-tooltip>关于</q-tooltip>
          <q-popup-proxy>
            <q-card class="shadow gsk-card" style="min-width: 320px">
              <q-card-section>
                <div class="text-h6 gsk-gradient-text">Gensokyo</div>
                <div class="text-caption text-muted q-mt-xs">幻想乡框架控制台</div>
              </q-card-section>
              <q-separator />
              <q-card-section class="text-body2">
                <p>基于 QQ 官方 API 的 OneBot V11 标准实现</p>
                <p class="text-muted" style="font-size: 0.85rem">
                  前端由 Quasar v{{ $q.version }} 驱动<br>
                  采用 Vercel + Linear 设计语言
                </p>
              </q-card-section>
              <q-separator />
              <q-card-actions align="right">
                <q-btn flat label="关闭" color="primary" v-close-popup size="sm" />
              </q-card-actions>
            </q-card>
          </q-popup-proxy>
        </q-btn>
      </q-toolbar>
    </q-header>

    <!-- Sidebar -->
    <q-drawer
      v-model="leftDrawerOpen"
      show-if-above
      bordered
      class="gsk-sidebar"
      :width="220"
      :breakpoint="700"
    >
      <div class="gsk-sidebar-inner">
        <!-- Logo area -->
        <div class="gsk-sidebar-header">
          <q-icon name="auto_awesome" size="28px" color="primary" />
          <div class="gsk-sidebar-brand">
            <div class="gsk-sidebar-title">Gensokyo</div>
            <div class="gsk-sidebar-subtitle">管理控制台</div>
          </div>
        </div>

        <q-separator class="gsk-sidebar-sep" />

        <!-- Navigation -->
        <q-list class="gsk-nav" dense>
          <q-item-label header class="gsk-nav-header">导航</q-item-label>
          <q-item clickable to="/index" exact class="gsk-nav-item">
            <q-item-section avatar class="gsk-nav-icon">
              <q-icon name="dashboard" size="20px" />
            </q-item-section>
            <q-item-section>
              <q-item-label>仪表盘</q-item-label>
            </q-item-section>
          </q-item>
          <q-item clickable to="/accounts/add" class="gsk-nav-item">
            <q-item-section avatar class="gsk-nav-icon">
              <q-icon name="add_circle" size="20px" />
            </q-item-section>
            <q-item-section>
              <q-item-label>添加机器人</q-item-label>
            </q-item-section>
          </q-item>
        </q-list>

        <q-separator class="gsk-sidebar-sep" />

        <!-- Accounts Section -->
        <q-list class="gsk-nav" dense>
          <q-item-label header class="gsk-nav-header">机器人列表</q-item-label>
          <AccountSelector />
        </q-list>
      </div>

      <!-- Sidebar Footer -->
      <div class="gsk-sidebar-footer">
        <q-separator />
        <div class="gsk-sidebar-footer-content">
          <span class="gsk-status-dot online"></span>
          <span class="text-caption text-muted">系统运行中</span>
        </div>
      </div>
    </q-drawer>

    <!-- Main Content -->
    <q-page-container class="gsk-page-container">
      <router-view v-slot="{ Component }" :key="$route.fullPath">
        <transition
          appear
          enter-active-class="animated fadeIn"
          leave-active-class="animated fadeOut"
        >
          <component :is="Component" />
        </transition>
      </router-view>
    </q-page-container>
  </q-layout>
</template>

<script setup lang="ts">
import AccountSelector from 'components/AccountSelector.vue';
import { ref } from 'vue';

const leftDrawerOpen = ref(true);

function toggleLeftDrawer() {
  leftDrawerOpen.value = !leftDrawerOpen.value;
}
</script>

<style lang="scss" scoped>
.gsk-layout {
  background: var(--gsk-surface-soft);
}

.gsk-header {
  background: var(--gsk-surface) !important;
  border-bottom: 1px solid var(--gsk-border);
  height: var(--gsk-header-height);
  box-shadow: var(--gsk-shadow-sm);
}

.gsk-toolbar {
  min-height: var(--gsk-header-height) !important;
  padding: 0 16px;
}

.gsk-toolbar-title {
  font-size: 0.95rem;
}

.fw-600 { font-weight: 600; }
.text-muted { color: var(--gsk-text-muted); }

// Sidebar
.gsk-sidebar {
  background: var(--gsk-surface) !important;
  border-right: 1px solid var(--gsk-border) !important;
}

.gsk-sidebar-inner {
  display: flex;
  flex-direction: column;
  height: calc(100vh - var(--gsk-header-height));
  padding-top: 0;
}

.gsk-sidebar-header {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 16px 20px;
}

.gsk-sidebar-brand {
  display: flex;
  flex-direction: column;
}

.gsk-sidebar-title {
  font-weight: 700;
  font-size: 1rem;
  background: var(--gsk-gradient-1);
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  background-clip: text;
}

.gsk-sidebar-subtitle {
  font-size: 0.75rem;
  color: var(--gsk-text-muted);
  -webkit-text-fill-color: var(--gsk-text-muted);
}

.gsk-sidebar-sep {
  margin: 0 12px;
  background: var(--gsk-border);
}

.gsk-nav-header {
  padding: 8px 20px 4px;
  font-size: 0.7rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--gsk-text-muted) !important;
  opacity: 0.8;
}

.gsk-nav-item {
  margin: 2px 8px;
  border-radius: 6px;
  transition: all 0.15s ease;

  &:hover {
    background: var(--gsk-surface-hover);
  }

  &.router-link-active,
  &.router-link-exact-active {
    background: rgba(99, 102, 241, 0.1);
    color: var(--gsk-primary);

    .gsk-nav-icon :deep(.q-icon) {
      color: var(--gsk-primary);
    }
  }
}

.gsk-nav-icon {
  min-width: 36px;
}

.gsk-sidebar-footer {
  margin-top: auto;
}

.gsk-sidebar-footer-content {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px 20px;
}

// Page container
.gsk-page-container {
  background: var(--gsk-surface-soft);
  min-height: calc(100vh - var(--gsk-header-height));
}
</style>
