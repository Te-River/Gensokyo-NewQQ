<template>
  <div class="gsk-sender">
    <div class="gsk-sender-header">
      <q-icon name="send" size="18px" color="primary" />
      <span class="gsk-sender-title">快速发消息</span>
    </div>
    <q-tabs v-model="sendType" dense class="gsk-sender-tabs" active-color="primary" indicator-color="primary">
      <q-tab name="group" label="群聊" icon="groups" size="sm" />
      <q-tab name="friend" label="好友" icon="chat" size="sm" />
    </q-tabs>
    <q-separator />
    <q-tab-panels v-model="sendType" animated class="gsk-sender-panels">
      <q-tab-panel name="group" class="q-pa-sm">
        <q-select
          v-model="selectedGroup"
          outlined
          dense
          @filter="(val, update) => getGroupList().then(r => update(() => groupList = r))"
          :options="groupList"
          option-label="group_name"
          option-value="group_id"
          label="选择群聊"
          map-options
          emit-value
          class="q-mb-sm"
          bg-color="transparent"
        >
          <template v-slot:option="scope">
            <q-item v-bind="scope.itemProps" dense>
              <q-item-section avatar>
                <q-avatar size="28px">
                  <q-img :src="`http://p.qlogo.cn/gh/${scope.opt.group_id}/${scope.opt.group_id}/100/`" />
                </q-avatar>
              </q-item-section>
              <q-item-section>
                <q-item-label class="text-body2">{{ scope.opt.group_name }}</q-item-label>
              </q-item-section>
            </q-item>
          </template>
        </q-select>
      </q-tab-panel>
      <q-tab-panel name="friend" class="q-pa-sm">
        <q-select
          v-model="selectedFriend"
          outlined
          dense
          @filter="(val, update) => getFriendList().then(r => update(() => friendList = r))"
          :options="friendList"
          option-label="nickname"
          option-value="user_id"
          label="选择好友"
          map-options
          emit-value
          class="q-mb-sm"
          bg-color="transparent"
        >
          <template v-slot:option="scope">
            <q-item v-bind="scope.itemProps" dense>
              <q-item-section avatar>
                <q-avatar size="28px">
                  <q-img :src="`https://q1.qlogo.cn/g?b=qq&nk=${scope.opt.user_id}&s=640`" />
                </q-avatar>
              </q-item-section>
              <q-item-section>
                <q-item-label class="text-body2">{{ scope.opt.nickname }}</q-item-label>
              </q-item-section>
            </q-item>
          </template>
        </q-select>
      </q-tab-panel>
    </q-tab-panels>
    <div class="gsk-sender-input-row">
      <q-input
        v-model="message"
        outlined
        dense
        autogrow
        placeholder="输入消息..."
        class="gsk-sender-input"
        bg-color="transparent"
      />
      <q-btn
        :disable="!message || !(selectedGroup || selectedFriend)"
        color="primary"
        icon="send"
        round
        size="sm"
        @click="doSend"
      />
    </div>
  </div>
</template>
<script setup lang="ts">
import { api } from 'src/boot/axios';
import { ref } from 'vue';

const props = defineProps({ uin: { type: Number, required: true } });

const sendType = ref<'group' | 'friend'>('group');
const message = ref<string>();
const groupList = ref<{ group_id: number; group_name: string }[]>([]);
const selectedGroup = ref<number>();
const friendList = ref<{ user_id: number; nickname: string }[]>([]);
const selectedFriend = ref<number>();

async function getGroupList() {
  const res = await api.accountApiApiUinApiPost(props.uin, 'get_group_list', { no_cache: true });
  const data = res.data as typeof groupList.value;
  groupList.value = data;
  return groupList.value;
}

async function getFriendList() {
  const res = await api.accountApiApiUinApiPost(props.uin, 'get_friend_list', { no_cache: true });
  const data = res.data as typeof friendList.value;
  friendList.value = data;
  return friendList.value;
}

async function sendMsg(message: string, options: { group_id: number } | { user_id: number }) {
  const res = await api.accountApiApiUinApiPost(props.uin, 'send_msg', { message, ...options });
  return res.data as { message_id: number };
}

async function doSend() {
  if (!message.value) return;
  const opts = selectedGroup.value
    ? { group_id: selectedGroup.value }
    : { user_id: selectedFriend.value! };
  await sendMsg(message.value, opts);
  message.value = undefined;
}
</script>

<style lang="scss" scoped>
.gsk-sender {
  border: 1px solid var(--gsk-border);
  border-radius: 12px;
  overflow: hidden;
}

.gsk-sender-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  font-size: 0.85rem;
  font-weight: 600;
  color: var(--gsk-text);
  border-bottom: 1px solid var(--gsk-border);
}

.gsk-sender-tabs {
  min-height: 36px;
  :deep(.q-tab) { min-height: 36px; padding: 4px 12px; }
}

.gsk-sender-panels {
  background: transparent;
}

.gsk-sender-input-row {
  display: flex;
  align-items: flex-end;
  gap: 8px;
  padding: 8px 12px 12px;
}

.gsk-sender-input {
  flex: 1;
}
</style>
