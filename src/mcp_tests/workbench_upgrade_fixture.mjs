import {project} from "./work_result_app_fixture.mjs";

// Synthetic canonical pages for the shipped controller and browser Host only.
export const goalId = "wc_goal_abcdefghijklmnop";
export const agentId = "wc_dagent_abcdefghijklmnop";
export const goalDetail = {
  summary:{goal_id:goalId,title:"Improve workspace",revision:3,lifecycle:"active"},
  objective:"Make workspace navigation useful",plan:{completion_conditions:["Directory navigation works"],steps:[]},
};
export const workbenchState = {
  project,
  settings:{instance_name:"WebCodex · development",display_name:"Personal workspace",display_name_override:"Personal workspace"},
  tasks:{tasks:[{task_id:"wc_agent_task_abcdefghijklmnop",title:"Review files",state:"active",execution_status:"active",created_at_unix_ms:1999999940000,latest_attempt:{started_at_unix_ms:1999999940000}}],total_count:1,next_offset:null,truncated:false},
  goals:{goals:[goalDetail.summary],next_offset:null,truncated:false},
  agents:{agents:[{agent_id:agentId,workbench_chat_available:true,production_auto_resume_available:true}],next_offset:null},
  execution_options:[{kind:"chat",agent_id:agentId,endpoint_id:"wc_endpoint_abcdefghijklmnop",expected_controller_generation:1},{kind:"runner",agent_id:agentId,provider_id:"default"}],
  providers:[{provider_id:"default",name:"Default"}],todos:[],capabilities:{manage_goals:true,submit_work:true,settings:true},errors:{},
};
