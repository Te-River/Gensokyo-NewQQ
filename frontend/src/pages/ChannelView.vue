<template>
  <q-page class="gsk-list-page">
    <div class="gsk-page-header">
      <div>
        <div class="gsk-page-title">子频道列表</div>
        <div class="gsk-page-subtitle">频道 ID: {{ channelid }}</div>
      </div>
    </div>

    <q-banner v-if="loading" class="q-mb-md" dense>
      <template v-slot:avatar><q-spinner color="primary" size="sm" /></template>
      加载中...
    </q-banner>
    <q-banner v-if="error" class="q-mb-md bg-negative text-white" dense>
      {{ errorMessage }}
    </q-banner>

    <q-card class="gsk-table-card">
      <ChannelList
        v-if="!loading && !error"
        :data-list="channelList"
        @select="handleSelectItem"
        @selectAll="handleSelectAll"
        @row-click="handleRowClick"
      />

      <div class="gsk-table-footer">
        <q-pagination v-model="currentPage" :max="totalPages" :max-pages="7" :boundary-numbers="false" size="sm" />
        <q-space />
        <q-btn :disabled="currentPage <= 1" icon="chevron_left" @click="previousPage" flat round size="sm" />
        <q-btn :disabled="currentPage >= totalPages" icon="chevron_right" @click="nextPage" flat round size="sm" />
      </div>
    </q-card>

    <q-card class="gsk-msg-card">
      <div class="gsk-msg-row">
        <q-input v-model="message" label="发送消息" outlined dense class="gsk-msg-input" bg-color="transparent" />
        <q-btn :disabled="!selectedItems.length || !message" label="发送" @click="sendMessage" color="primary" unelevated no-caps />
      </div>
    </q-card>
  </q-page>
</template>
<script setup lang="ts">
/* eslint-disable @typescript-eslint/no-unsafe-assignment */
import { ref, watch, reactive, computed, onMounted } from 'vue';
import { api } from 'src/boot/axios';
import GroupList from 'src/components/GroupList.vue';
import ChannelList from 'src/components/ChannelList.vue';
import { useQuasar } from 'quasar';
const props = defineProps<{ uin: number; channelid: string }>();
// Reactive state
const groupList = ref([]);
const channelList = ref([]);
const selectedItems = ref<string[]>([]);
const message = ref('');
const loading = ref(false);
const error = ref(false);
const $q = useQuasar();

// Computed for error message to make it reactive
const errorMessage = computed(() => {
  return error.value ? `获取子频道数据失败，请稍后再试。` : '';
});

// 分页状态和逻辑
const currentPage = ref(1);
const totalPages = ref(1000);
const pager = reactive({
  Before: props.channelid,
  After: props.channelid,
  Limit: '100', // 假设每页30条
});

// Fetch data based on type
async function fetchData(): Promise<void> {
  loading.value = true;
  error.value = false; // 重置错误状态
  try {
    const response = await api.accountApiApiUinApiPost(
      props.uin,
      'get_channel_list', // 固定 API 名称
      { ...pager } // 使用 pager 作为请求体发送
    );

    // 从响应中解构 data 和 totalPages
    const { data } = response;
    // 设置频道列表
    channelList.value = (data as { data: any[] }).data;
    console.error(channelList.value);

    totalPages.value = 1000; // 假设后端会返回总页数
  } catch (e) {
    error.value = true; // 设置错误状态
    console.error(e); // 输出错误到控制台
  } finally {
    loading.value = false;
  }
}

// 更新pager以获取下一页
const getNextPage = async (lastItemId: string) => {
  // 设置after为最后一个item的id，before清空
  pager.After = props.channelid;
  pager.Before = props.channelid;
  // 可以调用fetchDataByType来获取下一页的数据
  await fetchData();
};

// 更新pager以获取上一页
const getPreviousPage = async (firstItemId: string) => {
  // 设置before为第一个item的id，after清空
  pager.Before = props.channelid;
  pager.After = props.channelid;
  // 调用fetchDataByType来获取上一页的数据
  await fetchData();
};

// 下一页按钮的点击事件处理函数
const nextPage = async () => {
  if (currentPage.value < totalPages.value) {
    currentPage.value++;
    const lastItemId = String(
      channelList.value[channelList.value.length - 1].id
    );
    await getNextPage(lastItemId).catch((e) => console.error(e));
  }
};

// 当用户选择单个项目时调用
const handleSelectItem = (selectedItemId: string) => {
  // 更新响应式状态以反映当前选中的项目
  selectedItems.value = [selectedItemId];
  // 此外，可能还需要执行其他操作，比如发送请求或更新 UI 等
  // ...其他逻辑
};

// 当用户选择所有项目或取消选择所有项目时调用
const handleSelectAll = (selectedItemIds: string[]) => {
  // 更新响应式状态以反映当前选中的所有项目
  selectedItems.value = selectedItemIds;
  // 此外，可能还需要执行其他操作，比如批量处理或批量请求等
  // ...其他逻辑
};

// 上一页按钮的点击事件处理函数
const previousPage = async () => {
  if (currentPage.value > 1) {
    currentPage.value--;
    const firstItemId = String(channelList.value[0].id);
    await getPreviousPage(firstItemId).catch((e) => console.error(e));
  }
};

async function sendGuildChannelMessage(
  message: string,
  options: { id: number } // 只使用 id，因为您说要替换 user_id
) {
  const { data } = await api.accountApiApiUinApiPost(
    props.uin,
    'send_guild_channel_message',
    {
      message,
      ...options,
    }
  );
  return data as { message_id: number }; // 假设返回的数据结构中包含 message_id
}
const sendMessage = async () => {
  try {
    loading.value = true; // 开始加载
    const selectedIds = selectedItems.value.map((item: { id: number }) => ({
      id: item.id,
    }));
    for (const options of selectedIds) {
      const responseData = await sendGuildChannelMessage(
        message.value,
        options
      );
      console.log(`Message sent with ID: ${responseData.message_id}`);
    }
    message.value = ''; // 清空消息输入框
  } catch (e) {
    console.error('发送消息失败:', e);
    error.value = true;
    errorMessage.value =
      e instanceof Error ? e.message : '发送消息时发生未知错误';
    // 使用Quasar Notify插件来显示错误通知
    $q.notify({
      color: 'negative',
      position: 'top',
      message: errorMessage.value,
      icon: 'report_problem',
    });
  } finally {
    loading.value = false; // 结束加载
  }
};

onMounted(async () => {
  try {
    await fetchData();
  } catch (e) {
    console.error(e);
  }
});
</script>

<style lang="scss" scoped>
.gsk-list-page {
  padding: 24px;
  max-width: 1400px;
  margin: 0 auto;
}

.gsk-page-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  margin-bottom: 16px;
}

.gsk-page-title { font-size: 1.25rem; font-weight: 700; color: var(--gsk-text); }
.gsk-page-subtitle { font-size: 0.85rem; color: var(--gsk-text-muted); }

.gsk-table-card {
  border: 1px solid var(--gsk-border);
  border-radius: 12px;
  overflow: hidden;
  margin-bottom: 16px;
}

.gsk-table-footer {
  display: flex;
  align-items: center;
  padding: 8px 16px;
  border-top: 1px solid var(--gsk-border);
}

.gsk-msg-card {
  border: 1px solid var(--gsk-border);
  border-radius: 12px;
  overflow: hidden;
}

.gsk-msg-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px 16px;
}

.gsk-msg-input {
  flex: 1;
}
</style>
  