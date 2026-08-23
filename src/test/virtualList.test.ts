import { describe, expect, it } from 'vitest';
import { getVirtualWindow } from '../lib/utils/virtualList';

describe('getVirtualWindow', () => {
  it('mounts only the visible rows plus overscan', () => {
    expect(getVirtualWindow(10_000, 64, 640, 320, 2)).toEqual({
      start: 8,
      end: 17,
      offsetTop: 512,
      offsetBottom: 638_912,
    });
  });

  it('clamps the window and spacers at the end of a list', () => {
    expect(getVirtualWindow(20, 50, 900, 200, 3)).toEqual({
      start: 15,
      end: 20,
      offsetTop: 750,
      offsetBottom: 0,
    });
  });

  it('handles empty lists and invalid measurements safely', () => {
    expect(getVirtualWindow(0, 0, Number.NaN, Number.NaN)).toEqual({
      start: 0,
      end: 0,
      offsetTop: 0,
      offsetBottom: 0,
    });
  });
});
