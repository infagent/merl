#!/usr/bin/python3
"""Synthetic translator for the local mock provider, never a real integration."""
import json
import os
import sys

r = json.load(sys.stdin)
assert os.listdir('/input') == []
assert not any(name.endswith('_KEY') for name in os.environ)
assert not os.path.exists('/tmp/provider-state')
with open('/tmp/provider-state', 'w') as state:
    state.write('synthetic state must not cross encode/decode invocations')
assert r['schema'] == 'merl.provider-adapter-request/v1'
if r['kind'] == 'encode':
    assert 0 < r['settings']['max_output_tokens'] <= 50
    out = {
        'schema': 'merl.provider-adapter-response/v1', 'kind': 'encoded',
        'body': {'model': r['settings']['model'], 'messages': r['messages'],
                 'max_tokens': r['settings']['max_output_tokens']},
    }
else:
    assert r['kind'] == 'decode'
    body = r['body']
    out = {
        'schema': 'merl.provider-adapter-response/v1', 'kind': 'success',
        'text': body['text'], 'usage': body.get('usage'),
        'model': body['model'], 'request_id': body['request_id'],
    }
    if r['status'] >= 400:
        out = {
            'schema': 'merl.provider-adapter-response/v1', 'kind': 'failure',
            'code': 'rate_limited', 'usage': body['usage'],
            'model': body['model'], 'request_id': body['request_id'],
        }
json.dump(out, sys.stdout)
