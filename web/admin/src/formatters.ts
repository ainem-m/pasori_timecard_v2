import { format } from 'date-fns';

import type { AttendanceDay, AttendanceRequest, PunchEvent } from './types';

export function formatPunchEventLabel(eventType: PunchEvent['event_type']) {
  switch (eventType) {
    case 'clock_in':
      return '出勤';
    case 'clock_out':
      return '退勤';
    case 'break_start':
      return '休憩開始';
    case 'break_end':
      return '休憩終了';
    case 'temporary_out':
      return '一時外出';
    case 'temporary_return':
      return '戻り';
    case 'manual_correction':
      return '修正';
  }
}

export function formatMinutes(totalMinutes: number) {
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  return `${hours}時間 ${minutes}分`;
}

export function buildCurrentYearMonth() {
  return format(new Date(), 'yyyy-MM');
}

export function formatAttendanceStatus(status: AttendanceDay['status']) {
  switch (status) {
    case 'unconfirmed':
      return '未確認';
    case 'confirmed':
      return '確認済み';
    case 'locked':
      return '締め済み';
  }
}

export function formatAttendanceRequest(request: AttendanceRequest) {
  let payload: Record<string, unknown> = {};
  if (request.requested_payload_json) {
    try {
      const parsed = JSON.parse(request.requested_payload_json);
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
        payload = parsed as Record<string, unknown>;
      }
    } catch {
      payload = {};
    }
  }

  const payloadDate = typeof payload.date === 'string' ? payload.date : undefined;
  const payloadReason = typeof payload.reason === 'string' ? payload.reason : undefined;
  const payloadTime = typeof payload.time === 'string' ? payload.time : undefined;
  const payloadTarget = typeof payload.target === 'string' ? payload.target : undefined;

  return {
    targetDate: request.target_date || payloadDate || '-',
    title: request.request_type || '打刻修正',
    detail: request.reason || payloadReason || [payloadTarget, payloadTime].filter(Boolean).join(' ') || '理由は未入力です。',
  };
}
