<template>
  <q-page class="gsk-editor-page">
    <div class="gsk-editor-container">
      <div class="gsk-editor-header">
        <q-btn @click="$router.back" flat round icon="arrow_back" size="sm" />
        <q-icon name="smartphone" size="22px" color="accent" class="q-ml-sm" />
        <span class="gsk-editor-title">编辑设备信息</span>
        <q-space />
        <q-btn flat color="primary" icon="save" label="保存" @click="updateConfig" size="sm" no-caps unelevated />
        <q-btn flat color="secondary" icon="refresh" label="重新加载" @click="loadConfig" size="sm" no-caps />
        <q-btn flat color="negative" icon="delete" label="重置" @click="deleteConfig" size="sm" no-caps />
        <q-separator vertical class="q-mx-sm" />
        <q-btn flat color="primary" icon="login" label="导入 QDVC" @click="importDialog = true" size="sm" no-caps />
        <q-btn flat color="secondary" icon="logout" label="导出 QDVC" @click="exportDialog = true" size="sm" no-caps />
      </div>

      <!-- Export Dialog -->
      <q-dialog v-model="exportDialog">
        <q-card class="gsk-dialog">
          <q-card-section class="gsk-dialog-header">
            <q-icon name="logout" size="20px" color="primary" />
            <span>导出 QDVC</span>
          </q-card-section>
          <q-card-section>
            <q-input v-model="qdvcUri" :loading="qdvcUri.length <= 0" readonly type="textarea" label="QDVC 分享链接" outlined dense bg-color="transparent" />
          </q-card-section>
          <q-card-actions class="justify-center q-gutter-sm">
            <q-btn-toggle v-model="qdvcEncoding" toggle-color="primary" flat dense
              :options="[{ label: 'Base64', value: 'base64' }, { label: 'Base16384', value: 'base16384' }]" />
            <q-btn flat color="primary" icon="content_copy" label="复制" @click="writeQdvcUri" size="sm" no-caps />
          </q-card-actions>
        </q-card>
      </q-dialog>

      <!-- Import Dialog -->
      <q-dialog v-model="importDialog">
        <q-card class="gsk-dialog">
          <q-card-section class="gsk-dialog-header">
            <q-icon name="login" size="20px" color="primary" />
            <span>导入 QDVC</span>
          </q-card-section>
          <q-card-section>
            <q-input v-model="qdvcUri" :loading="qdvcApplying" :disable="qdvcApplying" outlined dense
              :rules="[(val) => QDVC.RE.test(val) || '不是有效的 QDVC 链接']" type="textarea" label="QDVC 分享链接" bg-color="transparent" />
          </q-card-section>
          <q-card-actions class="justify-center">
            <q-btn flat color="primary" icon="login" label="应用" @click="applyQdvcUri" size="sm" no-caps />
          </q-card-actions>
        </q-card>
      </q-dialog>

      <div class="gsk-editor-body">
        <config-file-editor
          v-if="typeof content !== 'undefined'"
          v-model="content"
          language="json"
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
import type { DeviceInfo } from 'src/api';
import { api } from 'boot/axios';
import { QDVC } from './qdvc-utils';

const $q = useQuasar();

const props = defineProps<{ uin: number }>();
const content = ref<string>();
const loading = ref(true);
const exportDialog = ref(false);
const importDialog = ref(false);
const qdvcUri = ref('');
const qdvcApplying = ref(false);
const qdvcEncoding = ref<'base64' | 'base16384'>('base64');

async function loadConfig() {
  try {
    loading.value = true;
    const { data } = await api.accountDeviceReadApiUinDeviceGet(props.uin);
    content.value = JSON.stringify(data, null, 2);
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
    content.value = JSON.stringify(
      await api.accountDeviceWriteApiUinDevicePatch(props.uin, JSON.parse(content.value) as DeviceInfo).then(r => r.data),
      null, 2
    );
    $q.notify({ message: '设备信息修改成功', color: 'positive' });
  } catch (e) {
    $q.notify({ message: `设备信息修改失败: ${(e as Error).message}`, color: 'negative' });
  } finally {
    loading.value = false;
  }
}

async function deleteConfig() {
  try {
    loading.value = true;
    await api.accountConfigDeleteApiUinConfigDelete(props.uin);
    await loadConfig();
    $q.notify({ message: '设备信息删除成功', color: 'positive' });
  } catch {
    $q.notify({ message: '设备信息删除失败', color: 'negative' });
  } finally {
    loading.value = false;
  }
}

onMounted(loadConfig);

// eslint-disable-next-line @typescript-eslint/no-unsafe-call
watch(exportDialog, async (val) => {
  if (val)
    try {
      qdvcUri.value = '';
      const device = await api
          .accountDeviceReadApiUinDeviceGet(props.uin)
          .then(({ data }) => JSON.stringify(data)),
        session = await api
          .accountSessionReadApiUinSessionGet(props.uin)
          .then(({ data }) => QDVC.decodeBase64(data.base64_content, false))
          .catch(() => undefined);
      qdvcUri.value = QDVC.stringify({ device, session }, qdvcEncoding.value);
    } catch (e) {
      $q.notify({
        message: `设备信息导入失败: ${(e as Error).message}`,
        color: 'negative',
      });
    }
  else qdvcUri.value = '';
});

async function writeQdvcUri() {
  if (navigator.clipboard) {
    await navigator.clipboard.writeText(qdvcUri.value);
    $q.notify({ message: '已复制到剪贴板', color: 'positive' });
  }
}

async function applyQdvcUri() {
  const parsed = QDVC.parse(qdvcUri.value);
  if (!parsed) return;
  try {
    qdvcApplying.value = true;
    if (parsed.device)
      await api.accountDeviceWriteApiUinDevicePatch(
        props.uin,
        JSON.parse(parsed.device) as DeviceInfo
      );
    if (parsed.session)
      await api.accountSessionWriteApiUinSessionPatch(props.uin, {
        base64_content: QDVC.encodeBase64(parsed.session),
      });
    $q.notify({ message: '设备信息导入成功', color: 'positive' });
  } catch (e) {
    $q.notify({
      message: `设备信息导入失败: ${(e as Error).message}`,
      color: 'negative',
    });
  } finally {
    qdvcApplying.value = false;
  }
}

// eslint-disable-next-line @typescript-eslint/no-unsafe-call
watch(qdvcEncoding, (val) => {
  const parsed = QDVC.parse(qdvcUri.value);
  qdvcUri.value = parsed ? QDVC.stringify(parsed, val as 'base64' | 'base16384') : '';
});
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
  flex-wrap: wrap;
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

.gsk-dialog {
  border-radius: 12px;
  min-width: 400px;
}

.gsk-dialog-header {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 1rem;
  font-weight: 600;
}
</style>
