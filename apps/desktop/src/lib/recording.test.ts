import { describe, expect, it } from 'vitest';
import { formatDuration, hostDecision, issueUrl, stopReasonMessage } from './recording';

describe('recording helpers', () => {
  it('builds an issue URL with only version and OS family', () => {
    const url = new URL(issueUrl('2.3.16', 'macos-aarch64'));
    expect(url.origin + url.pathname).toBe('https://github.com/D205-VN/Sanser/issues/new');
    expect([...url.searchParams.keys()].sort()).toEqual(['body', 'title']);
    expect(url.searchParams.get('body')).toContain('Sanser: 2.3.16');
    expect(url.searchParams.get('body')).toContain('OS: macos-aarch64');
  });

  it('strips anything personal or unexpected from the URL inputs', () => {
    const body = new URL(issueUrl('2.3.16 alice@example.com', '10.0.0.4 /Users/alice')).searchParams.get('body') ?? '';
    expect(body).not.toContain('@');
    expect(body).not.toContain('10.0.0.4');
    expect(body).not.toContain('/Users');
  });

  it('formats recording durations', () => {
    expect(formatDuration(0)).toBe('00:00');
    expect(formatDuration(65_400)).toBe('01:05');
    expect(formatDuration(3_726_000)).toBe('1:02:06');
  });

  it('maps stop reasons to messages and passes through other errors', () => {
    expect(stopReasonMessage('size-limit')).toBe('Recording stopped at the file size limit');
    expect(stopReasonMessage('user')).toBe('Recording saved');
    expect(stopReasonMessage('host-denied')).toBe('The host does not allow recording');
    expect(stopReasonMessage('Viewer is not running')).toBe('Viewer is not running');
  });

  it('answers host decisions from the Host setting', () => {
    expect(hostDecision(true, 'requested')).toBe('allow');
    expect(hostDecision(false, 'requested')).toBe('deny');
    expect(hostDecision(false, 'allowed')).toBe('deny');
    expect(hostDecision(true, 'allowed')).toBeNull();
    expect(hostDecision(true, 'none')).toBeNull();
  });
});
