import copy
import json
import os
from pathlib import Path
import tempfile
import subprocess
import unittest
from unittest.mock import patch
import external_observation_hook as hook
from platform_security import secure_created_path


class AdapterTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        (self.root / 'project').mkdir()
        (self.root / 'state').mkdir(mode=0o700)
        secure_created_path(self.root / 'state')
        self.config = dict(server_url='http://127.0.0.1:12345', authorization_file=str(self.root/'auth'),
            project='agent:fixture:project', project_root=str(self.root/'project'),
            workflow_session_id='wc_sess_fixture', local_session_id='local-fixture', state_dir=str(self.root/'state'))
        self.payload = dict(hook_event_name='PostToolUse', session_id='local-fixture', cwd=self.config['project_root'], tool_use_id='call-one', tool_name='Bash', tool_input={'command':'PRIVATE'}, tool_response='exit 0 PRIVATE')
        self.path = self.root/'config.json'; self.path.write_text(json.dumps(self.config)); secure_created_path(self.path)
        (self.root/'auth').write_text('Bearer fixture'); secure_created_path(self.root/'auth')

    def event(self):
        return hook.observation(self.config, self.payload)

    def test_capture_drops_private_payload_and_keeps_unknown(self):
        event = self.event()
        self.assertIsNone(event['exit_code'])
        self.assertNotIn('PRIVATE', json.dumps(event))
        self.assertEqual(event, self.event())
        self.payload['tool_use_id']='call-two'
        self.assertNotEqual(event['event_id'], self.event()['event_id'])

    def test_exact_binding(self):
        for key, value in [('session_id','other'),('cwd',str(self.root)),('tool_use_id',None)]:
            p=copy.deepcopy(self.payload);p[key]=value
            with self.assertRaises(hook.AdapterError):hook.observation(self.config,p)

    def test_other_hooks_are_noops(self):
        self.payload['hook_event_name']='Stop'
        self.assertIsNone(self.event())

    def test_config_must_be_private_and_outside_project(self):
        self.assertEqual(hook.load_config(self.path), self.config)
        if os.name == "nt":
            subprocess.run(
                ["icacls", str(self.path), "/grant", "*S-1-1-0:(R)"],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        else:
            self.path.chmod(0o644)
        with self.assertRaises(hook.AdapterError):hook.load_config(self.path)
        secure_created_path(self.path)
        self.config['state_dir']=self.config['project_root']
        self.path.write_text(json.dumps(self.config))
        with self.assertRaises(hook.AdapterError):hook.load_config(self.path)

    def test_remote_cleartext_rejected(self):
        self.config['server_url']='http://example.com'
        self.path.write_text(json.dumps(self.config))
        with self.assertRaises(hook.AdapterError):hook.load_config(self.path)

    def test_private_file_symlink_rejected(self):
        link=self.root/'linked'
        try:
            link.symlink_to(self.root/'auth')
        except OSError:
            if os.name == "nt":
                self.skipTest("Windows host does not permit file symlink creation")
            raise
        with self.assertRaises((OSError, hook.AdapterError)):hook.private_file(link)

    def test_ack_removes_exact_pending(self):
        sent=[]
        result=hook.deliver(self.config,self.event(),lambda c,e,t:sent.append(e))
        self.assertEqual(result['acknowledged'],1)
        self.assertEqual(sent,[self.event()])
        self.assertEqual(list((self.root/'state').glob('*.json')),[])

    def test_unknown_delivery_preserves_identity_and_retries_only_report(self):
        seen=[]
        def unavailable(c,e,t):
            seen.append(e); raise OSError('connection lost after request')
        with self.assertRaises(OSError):hook.deliver(self.config,self.event(),unavailable)
        self.assertEqual(len(list((self.root/'state').glob('*.json'))),1)
        hook.deliver(self.config,None,lambda c,e,t:seen.append(e))
        self.assertEqual(seen,[self.event(),self.event()])

    def test_config_change_does_not_retarget_pending(self):
        with self.assertRaises(OSError):hook.deliver(self.config,self.event(),lambda *a:(_ for _ in ()).throw(OSError()))
        self.config['workflow_session_id']='wc_sess_other'
        with self.assertRaisesRegex(hook.AdapterError,'association'):hook.deliver(self.config,None,lambda *a:self.fail('must not send'))

    def test_same_event_changed_payload_conflicts(self):
        with self.assertRaises(OSError):hook.deliver(self.config,self.event(),lambda *a:(_ for _ in ()).throw(OSError()))
        self.payload['tool_name']='Edit'
        with self.assertRaisesRegex(hook.AdapterError,'conflict'):hook.deliver(self.config,self.event(),lambda *a:self.fail('must not send'))

    def test_corrupt_pending_is_not_discarded(self):
        p=self.root/'state'/('a'*64+'.json');p.write_text('{');secure_created_path(p)
        with self.assertRaises(ValueError):hook.deliver(self.config,None,lambda *a:self.fail('must not send'))
        self.assertTrue(p.exists())

    def test_busy_is_visible_not_false_success(self):
        with hook.state_lock(self.config):
            with self.assertRaisesRegex(hook.AdapterError,'busy'):hook.deliver(self.config,self.event())

    def test_redirect_is_never_followed(self):
        self.assertIsNone(hook.NoRedirect().redirect_request(None,None,302,None,None,None))

    def test_http_request_and_matching_acknowledgement(self):
        event = self.event()
        report = {k: event[k] for k in ('adapter_id', 'event_id', 'exit_code')}
        report['tool'] = event['observed_tool']
        body = dict(success=True, output=dict(project=event['project'],
            session_id=event['session_id'], provenance='external_report', observation=report))
        class Response:
            def __enter__(self): return self
            def __exit__(self, *args): pass
            def read(self, n): return json.dumps(body).encode()
        with patch.object(hook.urllib.request, 'build_opener') as opener:
            opener.return_value.open.return_value = Response()
            hook.send(self.config, event, 1)
            request = opener.return_value.open.call_args.args[0]
            self.assertEqual(json.loads(request.data),
                {'tool': 'record_external_observation', 'params': event})
            self.assertNotIn('tool', json.loads(request.data)['params'])
            body['output']['observation']['tool'] = 'DifferentTool'
            with self.assertRaises(hook.AdapterError): hook.send(self.config, event, 1)

    def test_response_identity_checked_before_ack(self):
        class Response:
            def __enter__(self):return self
            def __exit__(self,*args):pass
            def read(self,n):return json.dumps({'success':True,'output':{'project':'other'}}).encode()
        with patch.object(hook.urllib.request,'build_opener') as opener:
            opener.return_value.open.return_value=Response()
            with self.assertRaises(hook.AdapterError):hook.send(self.config,self.event(),1)


if __name__=='__main__':unittest.main()
