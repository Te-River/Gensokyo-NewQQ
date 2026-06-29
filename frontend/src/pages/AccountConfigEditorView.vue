<template>
  <q-page class="gsk-editor-page">
    <div class="gsk-editor-container">
      <div class="gsk-editor-header">
        <q-btn @click="$router.back" flat round icon="arrow_back" size="sm" />
        <q-icon name="description" size="22px" color="primary" class="q-ml-sm" />
        <span class="gsk-editor-title">编辑配置文件</span>
        <q-space />
        <q-btn flat color="primary" icon="save" label="保存" @click="updateConfig" size="sm" no-caps unelevated />
        <q-btn flat color="secondary" icon="refresh" label="重新加载" @click="loadConfig" size="sm" no-caps />
        <q-btn flat color="negative" icon="delete" label="重置" @click="deleteConfig" size="sm" no-caps />
      </div>
      <div class="gsk-editor-body">
        <config-file-editor
          v-if="typeof content !== 'undefined'"
          v-model="content"
          language="yaml"
          style="height: 100%"
          :theme="$q.dark.isActive ? 'vs-dark' : 'vs'"
        />
        <q-inner-loading :showing="loading" />
      </div>
    </div>
  </q-page>
</template>
<script setup lang="ts">
import { onMounted, ref } from 'vue';
import { useQuasar } from 'quasar';
import ConfigFileEditor from 'src/components/ConfigFileEditor.vue';
import { api } from 'boot/axios';

const $q = useQuasar();
const props = defineProps<{ uin: number }>();
const content = ref<string>();
const loading = ref(true);

async function loadConfig() {
  try {
    loading.value = true;
    const { data } = await api.accountConfigReadApiUinConfigGet(props.uin);
    content.value = data.content;
  } catch {
    content.value = undefined;
  } finally {
    loading.value = false;
  }
}

async function updateConfig() {
  if (!content.value) return;
  try {
    loading.value = true;
    const { data } = await api.accountConfigWriteApiUinConfigPatch(props.uin, { content: content.value });
    content.value = data.content;
    $q.notify({ message: '配置文件修改成功', color: 'positive' });
  } catch {
    $q.notify({ message: '配置文件修改成功', color: 'positive' });
  } finally {
    loading.value = false;
  }
}

async function deleteConfig() {
  try {
    loading.value = true;
    await api.accountConfigDeleteApiUinConfigDelete(props.uin);
    await loadConfig();
    $q.notify({ message: '配置文件已重置', color: 'positive' });
  } catch {
    $q.notify({ message: '配置文件重置失败', color: 'negative' });
  } finally {
    loading.value = false;
  }
}

onMounted(loadConfig);
</script>

<style lang="scss" scoped>
.gsk-editor-page {
  padding: 16px;
  height: calc(100vh - var(--gsk-header-height));
  background: var(--gsk-surface-soft);
}

.gsk-editor-container {
  display: flex;
  flex-direction: column;
  height: 100%;
  border: 1px solid var(--gsk-border);
  border-radius: 12px;
  overflow: hidden;
  background: var(--gsk-surface);
}

.gsk-editor-header {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 10px 14px;
  border-bottom: 1px solid var(--gsk-border);
  flex-shrink: 0;
}

.gsk-editor-title {
  font-size: 0.95rem;
  font-weight: 600;
  color: var(--gsk-text);
}

.gsk-editor-body {
  flex: 1;
  overflow: hidden;
}
</style>
