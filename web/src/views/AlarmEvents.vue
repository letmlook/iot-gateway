<script setup>
import { useI18n } from 'vue-i18n'
import PageHeader from '../components/PageHeader.vue'
import AlarmEventList from '../components/AlarmEventList.vue'

const { t } = useI18n()
</script>

<template>
  <div class="alarm-events-page">
    <PageHeader :title="t('alarms.title')" :subtitle="t('alarms.desc')" />

    <div class="alarm-events-card">
      <AlarmEventList />
    </div>
  </div>
</template>

<style scoped>
.alarm-events-page {
  display: flex;
  flex-direction: column;
  min-height: calc(100vh - 120px);
}

.alarm-events-card {
  flex: 1;
  min-height: 0;
}
</style>
