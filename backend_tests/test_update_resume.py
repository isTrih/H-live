"""Update recovery uses isolated files and fake monitors, never real rooms."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from test_backend import FakeManager, wait_for
from backend_service import BackendService, VERSION, ServiceError
from app_storage import save_config
from update_resume import NAME, save_intent, read_intent, resume_rids, acknowledge


TARGET_VERSION=".".join([*VERSION.split(".")[:2],str(int(VERSION.split(".")[2])+1)])

class UpdateResumeTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.directory = Path(self.tmp.name)
        save_config({"anchors":{"111":"甲","222":"乙","333":"丙"},"proxy_pool":{"enabled":False},"first_run_complete":True}, self.directory)

    def service(self):
        service = BackendService(self.directory,manager_factory=FakeManager)
        self.addCleanup(service.shutdown)
        return service

    def test_intent_limits_versions_membership_and_contains_no_credentials(self):
        intent=save_intent(self.directory,VERSION,TARGET_VERSION,["111","111","222"])
        self.assertEqual(["111","222"],read_intent(self.directory)["rids"])
        self.assertEqual(["111"],resume_rids(intent,TARGET_VERSION,{"111":"甲"}))
        self.assertEqual([],resume_rids(intent,"99.0.0",{"111":"甲"}))
        self.assertEqual({"format","from_version","target_version","rids","nonce"},set(intent))
        for args in [("bad",TARGET_VERSION,[]),(VERSION,TARGET_VERSION,["0"]),(VERSION,TARGET_VERSION,["https://x"])]:
            with self.assertRaises(ValueError):save_intent(self.directory,*args)
        self.assertEqual(intent,read_intent(self.directory))

    def test_atomic_replace_failure_keeps_previous_and_cleans_temporary_file(self):
        original=save_intent(self.directory,VERSION,TARGET_VERSION,["111"])
        with patch("update_resume.os.replace",side_effect=OSError("fixture")):
            with self.assertRaises(OSError):save_intent(self.directory,VERSION,TARGET_VERSION,["222"])
        self.assertEqual(original,read_intent(self.directory))
        self.assertEqual([],list(self.directory.glob(".update-resume-*.tmp")))
        self.assertFalse(acknowledge(self.directory,"0"*32))
        self.assertTrue(acknowledge(self.directory,original["nonce"]))
        self.assertIsNone(read_intent(self.directory))

    def test_prepare_pauses_only_running_and_restart_resumes_only_those_once(self):
        service=self.service()
        service.manager.start("111","甲")
        service.manager.start("222","乙")
        service.manager.stop("222")
        result=service.dispatch("prepare_update",{"target_version":TARGET_VERSION})
        self.assertTrue(result["ready"])
        self.assertEqual(1,result["paused_tasks"])
        self.assertEqual(["222","111"],service.manager.stopped)
        self.assertEqual(["111"],read_intent(self.directory)["rids"])
        service.shutdown()
        newer=self.service()
        wait_for(lambda:newer._operations.unfinished_tasks==0)
        self.assertEqual(["111"],[row[0] for row in newer.manager.started])
        self.assertIsNone(read_intent(self.directory))
        newer.dispatch("cancel_update")
        self.assertEqual(1,len(newer.manager.started))

    def test_target_version_can_restore_and_removed_task_stays_removed(self):
        save_intent(self.directory,"0.2.1",VERSION,["111","999"])
        service=self.service()
        wait_for(lambda:service._operations.unfinished_tasks==0)
        self.assertEqual(["111"],[row[0] for row in service.manager.started])
        self.assertIsNone(read_intent(self.directory))

    def test_failed_installer_cancel_restores_in_current_version_without_duplicate_queue(self):
        service=self.service();service.manager.start("111","甲")
        self.assertTrue(service.dispatch("prepare_update",{"target_version":TARGET_VERSION})["ready"])
        service.dispatch("cancel_update");service.dispatch("cancel_update")
        wait_for(lambda:service._operations.unfinished_tasks==0)
        self.assertEqual(2,len(service.manager.started))
        self.assertIsNone(read_intent(self.directory))

    def test_save_failure_does_not_stop_monitor_and_busy_export_does_not_write_intent(self):
        service=self.service();service.manager.start("111","甲")
        with patch("update_resume.save_intent",side_effect=OSError("fixture")):
            with self.assertRaises(ServiceError):service.dispatch("prepare_update",{"target_version":TARGET_VERSION})
        self.assertTrue(service.manager.snapshot()["111"]["active"])
        self.assertEqual([],service.manager.stopped)
        service._jobs["fixture"]={"status":"running"}
        self.assertFalse(service.dispatch("prepare_update",{"target_version":TARGET_VERSION})["ready"])
        service._jobs.clear()
        self.assertIsNone(read_intent(self.directory))

    def test_malformed_or_unrelated_intent_is_preserved_without_starting_tasks(self):
        path=self.directory/NAME
        path.write_text('{"password":"fixture"}',encoding="utf-8")
        service=self.service()
        self.assertEqual([],service.manager.started)
        self.assertTrue(path.exists())
        service.shutdown()
        save_intent(self.directory,"0.8.0","0.9.0",["111"])
        service=self.service()
        self.assertEqual([],service.manager.started)
        self.assertTrue(path.exists())

    def test_start_failure_keeps_intent_for_next_launch(self):
        service=self.service();service.manager.start("111","甲")
        self.assertTrue(service.dispatch("prepare_update",{"target_version":TARGET_VERSION})["ready"])
        with patch.object(service.manager,"start",side_effect=RuntimeError("fixture")):
            service.dispatch("cancel_update")
            wait_for(lambda:service._operations.unfinished_tasks==0)
        self.assertEqual(["111"],read_intent(self.directory)["rids"])
        service.shutdown()
        next_launch=self.service()
        wait_for(lambda:next_launch._operations.unfinished_tasks==0)
        self.assertIsNone(read_intent(self.directory))
