<template>
  <q-page class="gsk-login-page">
    <div class="gsk-login-container">
      <!-- Brand -->
      <div class="gsk-login-brand">
        <q-icon name="hub" size="36px" color="primary" />
        <div class="gsk-login-brand-text">
          <span style="font-size: 1.5rem; font-weight: 700">Gensokyo</span>
          <span class="text-muted" style="font-size: 0.85rem">管理控制台</span>
        </div>
      </div>

      <!-- Login Card -->
      <q-card class="gsk-login-card">
        <q-card-section class="q-pb-none">
          <div class="text-h6" style="font-weight: 600">欢迎回来</div>
          <div class="text-caption text-muted q-mt-xs">请登录以继续管理你的机器人</div>
        </q-card-section>

        <q-card-section>
          <q-form
            autocorrect="off"
            autocapitalize="off"
            autocomplete="off"
            spellcheck="false"
            @submit.prevent="login"
            @reset="clearForm"
          >
            <q-input
              v-model="username"
              outlined
              clearable
              label="用户名"
              required
              class="q-mb-md"
              bg-color="transparent"
            >
              <template v-slot:prepend><q-icon name="person" color="primary" size="20px" /></template>
            </q-input>

            <q-input
              v-model="password"
              type="password"
              outlined
              clearable
              label="密码"
              required
              class="q-mb-lg"
              bg-color="transparent"
            >
              <template v-slot:prepend><q-icon name="lock" color="primary" size="20px" /></template>
            </q-input>

            <q-btn
              label="登录"
              type="submit"
              color="primary"
              class="full-width gsk-login-btn"
              :disable="!username || !password"
              no-caps
            />
            <q-btn
              label="清除"
              type="reset"
              flat
              class="full-width q-mt-sm"
              color="grey-6"
              no-caps
              size="sm"
            />
          </q-form>
        </q-card-section>
      </q-card>
    </div>
  </q-page>
</template>

<script setup lang="ts">
  import { api } from 'boot/axios';
  import { ref, onMounted } from 'vue';
  import { useRouter } from 'vue-router';
  import { useQuasar } from 'quasar';

  const $router = useRouter();
  const isLoggedIn = ref(false);
  const username = ref('');
  const password = ref('');
  const $q = useQuasar();

  async function checkLoggedIn() {
    try {
      const { data } = await api.checkLoginStatus();
      isLoggedIn.value = data.isLoggedIn;
      if (isLoggedIn.value) {
        void $router.push('/index');
      }
    } catch (err) {
      console.error('Error checking login status:', err);
      isLoggedIn.value = false;
    }
  }

  function clearForm() {
    username.value = '';
    password.value = '';
  }

  async function login() {
    if (!username.value || !password.value) return;
    try {
      const { data } = await api.loginApi(username.value, password.value);
      if (data.isLoggedIn) {
        isLoggedIn.value = true;
        void $router.push('/index');
      } else {
        $q.notify({
          color: 'negative',
          position: 'top',
          message: '登录失败，请检查用户名和密码。',
          icon: 'report_problem'
        });
      }
    // eslint-disable-next-line @typescript-eslint/no-unused-vars
    } catch (_err) {
      $q.notify({
        color: 'negative',
        position: 'top',
        message: '登录失败，请检查用户名和密码。',
        icon: 'report_problem'
      });
    }
  }

  onMounted(() => {
    checkLoggedIn().catch(error => {
      console.error('Failed to check login status:', error);
    });
  });
</script>

<style lang="scss" scoped>
.gsk-login-page {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 100vh;
  overflow: hidden;
  background: var(--gsk-surface-soft);
}

.gsk-login-container {
  position: relative;
  z-index: 1;
  width: 100%;
  max-width: 380px;
  padding: 20px;
}

.gsk-login-brand {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 32px;
  justify-content: center;
}

.gsk-login-brand-text {
  display: flex;
  flex-direction: column;
}

.text-muted { color: var(--gsk-text-muted); }

.gsk-login-card {
  border: 1px solid var(--gsk-border);
  border-radius: var(--gsk-radius);
  background: var(--gsk-surface);
  box-shadow: var(--gsk-shadow);
  overflow: hidden;
}

.gsk-login-btn {
  height: 44px;
  border-radius: var(--gsk-radius);
  font-weight: 600;
  text-transform: none;
}
</style>
