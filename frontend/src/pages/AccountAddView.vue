<template>
  <q-page class="gsk-add-page">
    <div class="gsk-add-container">
      <div class="gsk-add-header">
        <q-icon name="add_circle" size="32px" color="primary" />
        <div>
          <div class="gsk-add-title">添加机器人</div>
          <div class="gsk-add-subtitle">输入 AppID 创建一个新的机器人实例</div>
        </div>
      </div>

      <q-card class="gsk-add-card">
        <q-card-section>
          <q-form
            autocorrect="off"
            autocapitalize="off"
            autocomplete="off"
            spellcheck="false"
            @submit="addAccount"
            @reset="clearForm"
          >
            <q-input
              v-model.number="uin"
              autofocus
              outlined
              clearable
              label="AppID"
              :rules="[(v) => +v >= 1e4 || '请输入有效的 AppID']"
              class="q-mb-md"
              bg-color="transparent"
            >
              <template v-slot:prepend><q-icon name="badge" color="primary" /></template>
            </q-input>

            <div class="row q-gutter-sm">
              <q-btn type="submit" color="primary" icon="add" label="提交" no-caps unelevated class="gsk-btn" />
              <q-btn type="reset" flat color="grey-6" icon="clear" label="清除" no-caps />
            </div>
          </q-form>
        </q-card-section>
      </q-card>
    </div>
  </q-page>
</template>

<script setup lang="ts">
import { ref } from 'vue';
import { useQuasar } from 'quasar';
import { useRouter } from 'vue-router';
import { api } from 'src/boot/axios';

const $q = useQuasar(),
      $router = useRouter();

const uin = ref<number>();

async function addAccount() {
  if (!uin.value) return;
  try {
    $q.loading.show();
    await api.createAccountApiUinPut(uin.value, {});
    void $router.push(`/accounts/${uin.value}`);
  } catch (err) {
    $q.notify({
      color: 'negative',
      message: (err as Error).message,
    });
  } finally {
    $q.loading.hide();
  }
}

function clearForm() {
  uin.value = undefined;
}
</script>

<style lang="scss" scoped>
.gsk-add-page {
  display: flex;
  justify-content: center;
  padding: 40px 24px;
  min-height: calc(100vh - var(--gsk-header-height));
  background: var(--gsk-surface-soft);
}

.gsk-add-container {
  width: 100%;
  max-width: 480px;
}

.gsk-add-header {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 24px;
}

.gsk-add-title {
  font-size: 1.25rem;
  font-weight: 700;
  color: var(--gsk-text);
}

.gsk-add-subtitle {
  font-size: 0.85rem;
  color: var(--gsk-text-muted);
}

.gsk-add-card {
  border: 1px solid var(--gsk-border);
  border-radius: 12px;
}

.gsk-btn {
  border-radius: 8px;
  height: 40px;
}
</style>
