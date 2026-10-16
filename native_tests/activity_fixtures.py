"""Synthetic, explicitly tagged fixtures; these are NOT captured live packets.

Schema: jwwsjlm/douyinlive-proto@0d661be57c8f80947054b2ef0cd1ac2fdce3cc4b
protobuf/new_douyin.proto LotteryEventNewMessage / LuckyBoxMessage.
"""
import gzip


def varint(value):
    result = bytearray()
    while value > 127:
        result.append((value & 127) | 128)
        value >>= 7
    return bytes(result) + bytes([value])


def number(tag, value):
    return varint(tag << 3) + varint(value)


def blob(tag, value):
    return varint(tag << 3 | 2) + varint(len(value)) + value


def text(tag, value):
    return blob(tag, value.encode('utf-8'))


def lottery(room=200111, message=18446744073709551615):
    common = number(2, message) + number(3, room) + number(4, 1792022400123)
    condition = number(1, 99) + text(2, '合成口令') + number(4, 2)
    return (blob(1, common) + number(2, 18446744073709551614) + number(3, 99)
            + number(5, 1792022460) + number(13, 10) + number(14, 2)
            + text(18, '合成奖品') + text(19, '仅用于离线测试') + blob(21, condition))


def frame(messages, ack=True):
    response = b''.join(blob(1, text(1, method) + blob(2, payload)) for method, payload in messages)
    if ack:
        response += text(5, 'synthetic-ack') + number(9, 1)
    return number(2, 1) + blob(8, gzip.compress(response))
