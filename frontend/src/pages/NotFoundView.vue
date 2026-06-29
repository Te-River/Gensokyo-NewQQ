<template>
  <div class="gsk-not-found">
    <div class="gsk-404-content">
      <div class="gsk-404-number">404</div>
      <div class="gsk-404-text">页面未找到</div>
      <div class="gsk-404-desc">你访问的页面不存在或已被移除</div>
      <q-btn
        class="q-mt-md"
        color="primary"
        unelevated
        to="/index"
        label="返回仪表盘"
        no-caps
      />
    </div>
  </div>
</template>

<style lang="scss" scoped>
.gsk-not-found {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 80vh;
  background: var(--gsk-surface-soft);
}

.gsk-404-content {
  text-align: center;
}

.gsk-404-number {
  font-size: 8rem;
  font-weight: 800;
  background: var(--gsk-gradient-1);
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  background-clip: text;
  line-height: 1;
  margin-bottom: 8px;
}

.gsk-404-text {
  font-size: 1.5rem;
  font-weight: 600;
  color: var(--gsk-text);
  margin-bottom: 4px;
}

.gsk-404-desc {
  color: var(--gsk-text-muted);
  font-size: 0.9rem;
}
</style>
