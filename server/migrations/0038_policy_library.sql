-- Policy library: many HR-editable documents (the company handbook), readable
-- by every employee. Supersedes the single-document okf_document (0037): its one
-- row is migrated in as the System Rulebook, then that table is dropped.
CREATE TABLE IF NOT EXISTS okf_documents (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    slug         TEXT NOT NULL UNIQUE,
    title        TEXT NOT NULL,
    category     TEXT NOT NULL DEFAULT 'General',
    kind         TEXT NOT NULL DEFAULT 'markdown' CHECK (kind IN ('markdown','file')),
    content      TEXT NOT NULL DEFAULT '',
    storage_key  TEXT,
    file_name    TEXT,
    content_type TEXT,
    size_bytes   BIGINT,
    sort_order   INT NOT NULL DEFAULT 100,
    updated_by   UUID REFERENCES users (id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_okf_documents_sort ON okf_documents (category, sort_order, title);

-- Carry the existing single rulebook (0037) in as one entry.
INSERT INTO okf_documents (slug, title, category, kind, content, sort_order)
SELECT 'system-rulebook', 'Company Rulebook (System Config)', 'System', 'markdown', content, 0
FROM okf_document
ON CONFLICT (slug) DO NOTHING;

-- Seed the imported policy documents.
INSERT INTO okf_documents (slug, title, category, kind, content, sort_order) VALUES
  ('leave-attendance', 'Leave & Attendance Policy', 'Leave & Time', 'markdown', $okfdoc$# Leave & Attendance Policy

_Imported from the original policy document; HR can edit and reformat this in the portal._

hello@rapidinnovation.io
rapidinnovation.io
+91 6263954009
1st Floor, Tower-A, Bhutani Cyber
Park, C Block, Phase 2, Sector 62,
Noida, U.p - 201309
Policy
Leave and Attendance
Working Hours, Attendance, Leaves, Policy
Issue Date: 01st April 2026
Last Updated: 01st April 2026
Objective:
To establish an internal procedure for the preparation of attendance records, have control over
absenteeism of employees, facilitate payment of salary, and enable the employee to take leaves to
maintain a healthy work-life balance.
Scope:
This policy is applicable to all employees.
Attendance and Leave Policy:
The Leave and Attendance Policy is a crucial part of our company's HR policy, outlining rules for employee
attendance and leaves. It covers types of leaves, attendance procedures, the application process, and
consequences of violations. Our policy aims to create a fair and transparent system for effective workforce
management. This handbook presents our company's policy for employees' clarity and compliance. This
policy is designed to promote a positive and productive work environment by establishing clear guidelines
for employee attendance and working hours. It also encourages employees to prioritize their well-being
and maintain a healthy work-life balance by taking planned time off.
The intentions of creating this policy are as follows:
1.  A guideline to calculate employee attendance and accurate salary preparation
2.  Compliance with statutory guidelines
3.  Define office timing as per business requirements
4.  The Leave Accounting Period (LAP) is based on the Financial year (April-March)
5.  Eligible leaves will be credited to the employee account at the start of each month.
Monthly Paid Leave Limit
Effective 01st April 2026, employees are permitted a maximum of 2 paid leaves per calendar
month. This limit applies collectively across Sick Leave (SL), Earned Leave (EL), and Casual Leave
(CL).
a)  Any leave taken beyond 2 days in a month — regardless of type (SL, EL, or CL) — will be treated
as Loss of Pay (LOP).
b)  If an employee requires more than 2 leaves in a month, they must inform both their Reporting
Manager and HR at least 7 days in advance over email. Approval from both the Manager and
HR is mandatory.
c)  Even when such additional leave is approved, any leave beyond the 2 paid leaves in that month
will still be considered as LOP.

hello@rapidinnovation.io
rapidinnovation.io
+91 6263954009
1st Floor, Tower-A, Bhutani Cyber
Park, C Block, Phase 2, Sector 62,
Noida, U.p - 201309
Leave Type
Yearly Entitlement
Carry Forward / Lapse
Eligibility
EL (Earned Leave)
12 days (Pro-rata Basis)
10 leaves
Full-time employee
SL (Sick Leave)
6 days
Nil
All Employees
CL (Casual Leave)
6 days
Nil
Full-time employee
Special Leaves
5 working days
Nil
After 1 year of employment
Parental Leave
ML - 182 days / 82 days
Up to 2 children / More than 2
children
After 1 year of employment
APL/PL - 10 days
To be taken within 30 days from
the DOB
After 1 year of employment
AML (Adoption Maternity Leave)
- 84 days / 34 days
Child up to 3 months / Child
over 3 months
After 1 year of employment
Commissioning Mother 84 days
Up to 2 children
After 1 year of employment
Compensatory off
Granted when worked on
predefined Holiday or weekend
Nil
All Employees
Optional Leaves
2 days
Regional Festivals
All Employees
1) Earned Leaves
a)  Employees are entitled to 12 days of Earned Leave per financial year (April to March), accrued at a
rate of 1 day per month from the date of confirmation.
b)  EL is credited to the employee's leave account monthly, and the leave balance will be updated on
the first day of each month.
c)  Employees are required to apply for EL at least 10 working days in advance, subject to approval
from their reporting manager.
d)  A maximum of 10 days of unused EL can be carried forward to the next financial year. Any excess
leave beyond 10 days will be forfeited at the end of the financial year.
e)  In case of any unforeseen circumstances or emergencies where 10 days notice is not possible,
employees should inform their reporting manager and HR as soon as possible and apply for leave
through the designated leave management system. Approval for such short-notice leave requests will
be at the discretion of the reporting manager and HR, based on the nature of the emergency and the
impact on business operations.
f)  Employees on probation are not eligible for EL. They will start accruing EL from the date of their
confirmation.
g)  If an employee is on a notice period, they are not entitled to any earned leaves (ELs). However,
their existing EL balance will be encashed upon leaving the organization. Additionally, any leave taken
during the notice period will be treated as LOP and will extend the notice period by the corresponding
number of days.
h)  Employees are encouraged to plan their leave in advance to ensure a healthy work-life balance
and to minimize disruptions to business operations.
Example: If you take earned leave on May 1st and then request sick leave the following day, only the
earned leave will be granted. The sick leave request will not be approved, and you will have to take it as a
Loss of Pay (LOP).

hello@rapidinnovation.io
rapidinnovation.io
+91 6263954009
1st Floor, Tower-A, Bhutani Cyber
Park, C Block, Phase 2, Sector 62,
Noida, U.p - 201309
2) Sick Leave
a)  Employees are granted 6 days of sick leave per financial year, which will be added on the date of
joining.
b)  For absences due to illness lasting 3 or more consecutive days, a medical certificate from a
registered medical practitioner is required.
c)  If an employee has exhausted their SL balance and requires additional time off due to illness, they
may apply for Earned Leave (EL) with the approval of their reporting manager and HR.
d)  If both SL and EL balances are exhausted, employees may apply for Leave Without Pay (LWP) on
medical grounds, subject to approval from their reporting manager and HR.
e)  Unused SL will not be carried forward to the next financial year and will lapse at the end of the year.
f)  Employees on probation are eligible for SL from the date of joining.
Example: If an employee falls ill and requires 5 days of leave, but they have only 2 days of SL balance
remaining, the leave application will be processed as follows:
  2 days will be deducted from the employee's SL balance.
  The remaining 3 days will be deducted from the employee's EL balance, subject to availability and
approval from the reporting manager and HR.
3) Casual Leaves
a)  Employees are granted 6 days of Casual leave per financial year, which will be added on the date
of joining.
b)  Casual Leave can be availed for urgent & unforeseen matters which are not planned, usually, it is
the same day or a few days in advance.
c)  Only 2 Casual Leaves can be taken consecutively.
d)  Casual leave cannot be clubbed with any other type of leaves.
e)  Employees on probation are not eligible for CL. They will start accruing CL from the date of their
confirmation.
f)  The remaining CL will not be carried forward.
g)  Employees who are serving their notice period will not accrue CL during that period.
4) Special Leaves
a)  Rapid Innovation grants 5 days of Special Leave (SL2) to all confirmed employees who have
completed one year of service from their date of confirmation.
b)  SL2 is provided to support employees during the following auspicious occasions or situations:
i)  Employee's own marriage
ii)  Death of an immediate family member.
c)  For special events like marriage they need to apply for leave 30 days in advance.
d)  If an employee needs to extend their leave beyond the 5 days of SL2, they may club SL2 with
Earned Leave (EL) or apply for Leave Without Pay (LWP), subject to approval from their reporting
manager and HR.
e)  SL2 cannot be carried forward to the next year.
Example: If the employee needs 7 days of leave for their marriage and has an EL balance of 3 days, they
can avail 5 days of SL2 and club it with 2 days of EL, subject to approval. In case of the death of an

hello@rapidinnovation.io
rapidinnovation.io
+91 6263954009
1st Floor, Tower-A, Bhutani Cyber
Park, C Block, Phase 2, Sector 62,
Noida, U.p - 201309
immediate family member, the same process would apply.
5) Parental Leave
a)  Maternity Leave: If a female employee has worked for at least 80 days in the 12 months before
her expected date of delivery, she is entitled to maternity leave as follows:
i)  For the first two children: 182 days (26 weeks) of fully paid maternity leave.
ii)  For the third child and beyond: 84 days (12 weeks) of fully paid maternity leave.
iii)  To avail maternity leave, the employee must submit a certificate from a Registered Medical
Practitioner, indicating the expected date of delivery.
b)  Adoption Leave: If a female employee legally adopts a child below the age of three months, she is
entitled to 84 days (12 weeks) of fully paid adoption leave.
i)  If a female employee legally adopts a child aged three months or older, she is entitled to 30
days of fully paid adoption leave.
ii)  The period of adoption leave will be calculated from the date the child is handed over to the
adopting mother.
iii)  Adoption leave can be availed for a maximum of two children.
c)  Commissioning Mother: A commissioning mother (a woman who uses her egg to create an
embryo implanted in any other woman) is entitled to 84 days (12 weeks) of fully paid maternity leave
for up to two children. The period of maternity leave for a commissioning mother will be calculated
from the date the child is handed over to the commissioning mother.
d)  Paternity Leave: Male employees are entitled to 10 days of fully paid paternity leave. Paternity
leave must be taken within 30 days from the date of the child's birth.
e)  Additional Leave: In exceptional cases, a female employee can avail Privilege Leave/Exigency
Leave after exhausting the applicable Maternity Leave, subject to prior approval from the HR Head.
The employee must apply to their immediate manager for such additional leave.
Eligibility: Maternity Leave, Adoption Leave, and Paternity Leave are applicable only to employees who
have completed one year of service with Rapid Innovation.
6) Compensatory Leave (CO)
a)  Compensatory off (CO) is a leave granted by the Supervisor/Company Senior Official to an
employee for working on a weekend or a declared company holiday at the request of their Supervisor.
b)  The hours of working will be matched as per the hours of working on the Team Logger. Use of
Team Logger is mandatory for working extra on holidays and week-offs as per the requirement of the
project.
c)  COs are non-statutory leaves provided to compensate employees for the time they spend working
on weekends or holidays to meet business needs.
d)  COs will be credited to an employee's leave balance by the HR department, based on approval
from the respective supervisor.
e)  An employee can accumulate a maximum of 3 COs at any given time. Any COs earned beyond this
limit will be forfeited.
f)  CO should be utilised within 30 days of credit. In exceptional cases, where an employee is unable to
utilise their CO within the 30-day period due to business requirements or other compelling reasons, a
one-time extension of the validity period may be granted with the approval of the Supervisor and HR.

hello@rapidinnovation.io
rapidinnovation.io
+91 6263954009
1st Floor, Tower-A, Bhutani Cyber
Park, C Block, Phase 2, Sector 62,
Noida, U.p - 201309
g)  No more than 2 COs can be clubbed together to be availed at a time.
h)  CO cannot be carried forward beyond the specified validity period.
Comp off Computation:
  4 hours working would be considered as half day comp off.
  Less than 4 hours will be counted as a week off and no comp off will be applicable.
  8 hours and above working would be considered a full day comp off.
Important Points to Remember:
1.  As a responsible employee of Rapid Innovation, you are expected to use your leaves in a
pre-planned manner during the year.
2.  All leaves must be informed in advance. Unplanned leaves are highly discouraged.
3.  Employees are permitted a maximum of 2 paid leaves per calendar month (across SL, EL and CL
combined). Any leave beyond 2 in a month will be treated as Loss of Pay (LOP). Requests for more
than 2 leaves in a month require email approval from both the Reporting Manager and HR at least 7
days in advance, and the excess leaves will still be considered LOP. (Effective 01st April 2026)
4.  For planned leaves of more than 5 consecutive days, approval should be taken at least 3 weeks in
advance from the Team Lead by email and mark HR in cc.
5.  It is an employee's responsibility to apply the leave requests email without fail to the HR/reporting
manager and communicate the leave plan to all the team members for ready reference.
6.  It is the responsibility of the respective Reporting Manager to approve all the leaves on or before
the last day of every month on Razorpay.
7.  The Team Lead/Reporting Manager has full authority to reject the leave request of an employee
due to business requirements, so it is highly recommended that the leave plan should be discussed in
advance and fairly distributed among the team.
8.  We don't have a sandwich rule in Rapid Innovation, there are exceptions to the policy. For
example, if an employee takes planned leaves on Friday and Monday, only two earned leaves will be
counted. However, if an employee takes a leave on Wednesday and Thursday and also remains
absent from work on Friday, then the weekend (Saturday and Sunday) will also be deducted from their
earned leaves. This means that a total of five days' leave will be deducted.
9.  It is strictly forbidden for employees to take any unplanned leave, whether it is Sick or Earned
leave, on the day before or after a long holiday. For example, if January 1 falls on a Friday, employees
cannot take unplanned leave on December 31 (Thursday) or January 4 (Monday). If an employee is
too sick and needs to take leave, they must provide a doctor's prescription and apply for leave.
Otherwise, any unplanned leave (Sick or Earned) will be considered as Leave Without Pay.
$okfdoc$, 10),
  ('comp-off', 'Compensatory-Off Policy', 'Leave & Time', 'markdown', $okfdoc$# Compensatory-Off Policy

_Imported from the original policy document; HR can edit and reformat this in the portal._

CompensatoryOff (CO) Policy
Purpose
CompensatoryLeave(CompOff) isgrantedtoemployeestocompensatefor their extraworkhoursonweekendsor companyholidaysasrequestedbytheir supervisor.
ScopeandApplicability
Thispolicyappliestoall theCodezero2pi-confirmedemployeeswhoareworkingwithus.
KeyPoints
1. Thehoursof workwill bematchedasper thehoursof workontheTeamLogger. Theuseof TeamLogger ismandatoryfor workingextraonholidaysandweek-offsaspertherequirementsof theproject.2. COsarenon-statutoryleavesprovidedtocompensateemployeesfor thetimetheyspendworkingonweekendsor holidaystomeet businessneeds.3. COswill becreditedtoanemployee'sleavebalancebytheHRdepartment, basedonapproval fromtherespectivesupervisor.4. Anemployeecanaccumulateamaximumof 3COsat anygiventime. AnyCOsearnedbeyondthislimit will beforfeited.5. COshouldbeutilisedwithin30daysof credit. Inexceptional cases, whereanemployeeisunabletoutilisetheir COwithinthe30-dayperiodduetobusinessrequirementsor other compellingreasons, aone-timeextensionof thevalidityperiodmaybegrantedwiththeapproval of thesupervisor andHR.6. Amaximumof twoCOscanbecombinedat anygiventime.7. COcannot becarriedforwardbeyondthespecifiedvalidityperiod.
ComputedComputation
1. 4hoursof workingwouldbeconsideredahalf-dayof comptime.2. Lessthan4hourswill becountedasaweekoff, andnocompensationwill beapplicable.3. 8hoursandaboveworkingwouldbeconsideredafull daycompoff.
Guideline
1. Theproject manager hastomarkamail for thecompoff toHR.2. ToobtainCO, employeesneedtosendanemail totheir reportingmanager andtheTeamHR, andafter receivingapproval, theyshouldapplyfor thesameonRazorpay.3. Thecompensatoryleaveaccumulatedinonequarter hastobeusedinthat quarter,after whichit will lapse.4. Youcannot clubyour comp-off leaveswithanyother leave, i.e., sickleaves, earnedleaves, special leaves, or optional leaves.5. Compensatoryleaveswill beconsideredonlywhenyouareaskedtoworkonweekendsbyyour managersor HODs.
$okfdoc$, 11),
  ('gratuity', 'Gratuity Policy', 'Compensation & Benefits', 'markdown', $okfdoc$# Gratuity Policy

_Imported from the original policy document; HR can edit and reformat this in the portal._

Gratuity  Policy  
Purpose  
The  purpose  of  this  policy  is  to  define  the  eligibility,  calculation,  and  payment  process  of  
gratuity
 
to
 
employees
 
of
 
Rapid
 
Innovation
 
in
 
accordance
 
with
 
the
 
Payment
 
of
 
Gratuity
 
Act,
 
1972
,
 
and
 
any
 
amendments
 
thereto.
 
 
Scope  
This  policy  applies  to  all  eligible  employees  of  Rapid  Innovation,  including  permanent  
employees,
 
who
 
have
 
completed
 
the
 
minimum
 
required
 
period
 
of
 
continuous
 
service
 
as
 
prescribed
 
under
 
the
 
Act.
 
 
3.  Eligibility  Criteria  
An  employee  shall  be  eligible  for  gratuity  if:  
1.  The  employee  has  completed  five  (5)  years  of  continuous  service  with  the  
Company.
 
 
 2.  Gratuity  becomes  payable  on:  
 ○  Resignation  
 ○  Retirement  
 ○  Superannuation  
 ○  Termination  (other  than  for  misconduct  involving  moral  turpitude,  subject  to  
provisions
 
of
 
law)
 
 ○  Death  or  permanent  disability  
 
 

 
Definition  of  Continuous  Service  
Continuous  service  shall  be  defined  as  per  Section  2A  of  the  Payment  of  Gratuity  Act,  1972,  
including
 
uninterrupted
 
service
 
and
 
service
 
interrupted
 
due
 
to:
 
●  Sickness  
 ●  Accident  
 ●  Leave  
 ●  Layoff  
 ●  Strike  or  lockout  
 ●  Any  reason  not  attributable  to  the  employee  
 
 
Calculation  of  Gratuity  
Gratuity  shall  be  calculated  as  per  the  formula  prescribed  under  the  Act:  
Formula:  Gratuity=  (Last  Drawn  Salary  ×  15  ×  Number  of  Completed  Years  of  Service)  ÷  26   
Where:  
●  Last  Drawn  Salary  =  Basic  Salary  
 ●  15  =  15  days’  wages  for  every  completed  year  of  service  
 ●  26  =  Number  of  working  days  in  a  month  (as  per  Act)  
 ●  Total  Service  =  7  years  and  8  months  
 
Payment  Timeline  
●  Gratuity  shall  be  paid  with  FNF  or  within  30  days  from  the  date  it  becomes  payable.  
 
 

 
 
Forfeiture  of  Gratuity  
Gratuity  may  be  wholly  or  partially  forfeited  in  cases  of:  
●  Willful  damage  or  loss  caused  to  Company  property  (to  the  extent  of  damage/loss)  
 ●  Termination  for  riotous  or  disorderly  conduct  
 ●  Termination  for  any  act  involving  moral  turpitude  committed  during  employment  
 
Such  forfeiture  shall  be  in  accordance  with  Section  4(6)  of  the  Act.  
 
Tax  Treatment  
Gratuity  shall  be  subject  to  applicable  income  tax  laws.  Tax  exemption  shall  be  provided  as  
per
 
the
 
provisions
 
of
 
the
 
Income
 
Tax
 
Act,
 
1961.
 
  
Points  to  remember   ●  Gratuity  is  governed  by  the  Payment  of  Gratuity  Act,  1972,  and  statutory  provisions  
shall
 
prevail.
 
 ●  A  minimum  of  five  years  of  continuous  service  is  required  (except  in  cases  of  death  
or
 
permanent
 
disablement)
 
to
 
qualify
 
under
 
the
 
law.
 
 ●  Gratuity  is  calculated  on  the  last  drawn  Basic  Salary.   ●  The  maximum  payable  amount  and  tax  treatment  shall  be  as  per  applicable  
government
 
regulations.
 
 ●  Payment  shall  be  made  within  30  days,  and  forfeiture  is  permitted  only  under  
conditions
 
specified
 
under
 
the
 
Act.
$okfdoc$, 20),
  ('insurance-benefits', 'Insurance & Benefits', 'Compensation & Benefits', 'markdown', $okfdoc$# Insurance & Benefits

_Imported from the original policy document; HR can edit and reformat this in the portal._

“You now have one less thing to worry about as we’ve got 
you and your family covered for any unforeseen 
hospitalization expenses.”
Sponsored by
Codezero2Pi
Insurance Partner
ABHI
You would have received portal login details on your registered 
email id and phone no. If not, please inform your HR/Admin

We’ve covered you and your loved ones with a comprehensive health insurance 
protection from ABHI General Insurance.
Policy Benefits
Members Covered: 
Self, Spouse and Children
Sum Insured (SI): 
₹ 5,00,000
Policy Term
09-Jan-2025 to 08-Jan-2026
Co-pay: 0% for All
Hospitals Covered
• Cashless: Network Hosp.
• Reimb: All Hospitals across India
Coverage benefits
• In-patient treatments (min. 24 
hours hospitalization) (IPD)
• All day care treatments
• Pre-hospitalization: 30 days*
• Post-hospitalization: 60 days*
• Road Ambulance: ₹ 1,500/ claim
*medicines, lab tests & consultations
Disease Capping: Nil
Room Category Restriction
• Normal: ₹ 10,000 per life
• ICU: ₹ 20,000 per life
How am I covered?
Waiting Period
Initial Waiting : NIL
Specific Diseases : NIL
Pre-Existing Diseases : NIL
Special Features
Maternity Expenses : Normal - ₹50K; C-Section - ₹50K
Domiciliary  Hosp. : Covered up to 100 % of SI
Ayurvedic Treatments : Covered upto 25,000 in a government Hospital
Organ Donor Expenses : Medical expenses covered
Air Ambulance : Not Covered
Psychiatric Illnesses : Covered upto ₹30k (Min. 24 hours of in-patient 
hospitalization)
Others
• 50% Co-pay for Cyber Knife treatment, Gamma Knife treatment, Robotic Surgery, 
Stem Cell Transplantation
• Lasik treatment is covered if the power of the eye is greater than +/- 7.5

Note: The above list is illustrative and not an exhaustive list of 
exclusions / specific diseases covered from Day 1.
OPD Expenses External equipments
Diagnostic, pharmacy, lab tests 
where hospitalization is not 
required
Cost of spectacles, contact lenses, 
hearing aids, prosthesis
Miscellaneous charges Dental treatment
Admin, registration, and service 
charges
Dental surgeries of any kind unless 
requiring hospitalization in case of 
accidents
Infertility Treatments Foreign treatments
Infertility treatments such as IVF 
and surrogacy
Treatments taken outside India
Cosmetic/Plastic surgery Certain injectables
Plastic surgery unless necessary 
for treatment of a disease or as 
may be necessitated due to 
treatment of an accident
Avastin, ramicade and similar 
injectables
Miscellaneous Exclusions Non-Medical Consumables
Abortion, sleep disorder, 
external congenital diseases, 
AIDS, vaccination, drug abuse
Food, toiletries, television, laundry 
charges, gowns, masks, crepe 
bandages etc.
Specific Diseases INCLUDED from Day 1
1. Knee/Joint Replacement Surgery 6. Hysterectomy 
2. Sinus, Tonsils 7. Fissures
3. Kidney Stones 8. Hernia
4. Cataract Surgery 9. Varicose Veins 
5. Skin Tumours
Expenses/ Treatments permanently EXCLUDED
What are the inclusions/ exclusions?

Cashless Treatment
Cashless Facility can be availed from our network hospitals only. For list of network 
hospitals available, click here
Step 1: Claim Intimation
• Inform on Claims Helpline No (+91-90213 23456) for guidance on cashless 
process. 
• HealthySure team will support you with policy e-card copy in case you do not 
have it handy
Step 2: Claim Submission (Cashless Pre-Authorization Request)
• Arrange following documents and furnish to the hospital insurance desk on 
e-mail / in-person :
• The hospital insurance desk will verify the docs and seek pre-authorization 
directly from insurance co. Note: The hospital insurance desk will fill the pre-
authorization form on behalf of the patient
• Make sure the hospital insurance desk shares the completed form with ABHI
Step 3: Claim Settlement (Cashless Pre-Authorization Approval)
• ABHI team will review the case and will generate a ‘Claim Ref. No.’ with pre-
approved limit within 1 hour of valid submission 
Note: The approved limit would be a reasonable value to initiate the treatment. 
Any additional cost of treatment will be separately approved at time of 
discharge
• In case pre-authorization is not received, you may claim through reimbursement 
mode after discharge from the hospital
• Pre and post hospitalization bills for cashless treatments can be claimed through 
reimbursement mode after discharge from the hospital
Documents Required for Cashless
• Policy E-Card • Valid ID proof • Medical practitioner's 
referral letter advising 
hospitalization
How do I claim?

Reimbursement of claims can be availed across all hospitals in India (except if 
blacklisted by ABHI).
Step 1: Claim Intimation
• Inform on Claims Helpline No.(+91-90213 23456) for guidance on the 
reimbursement process
• Share basic details of hospitalization with the allocated manager within 48 hours 
post discharge. HealthySure to file a claim intimation with insurance company
Step 2 : Claim Submission
• Collate and send scanned copies (colored) of the following documents via email 
on care@healthysure.in . 
• For IPD and pre-hospitalization claims - Within 15 days of discharge
• For post hospitalization claims - Within 15 days of the date of discharge
• Healthysure’s Claim team to review all docs and share a pre-filled 
reimbursement form with you
Step 3: Claim Settlement
• ABHI team will review copies of documents submitted
• Claims will be settled within 30 days from the date of valid submission
Documents Required for Reimbursement
• Valid ID proof
• Medical practitioner's 
referral letter advising 
hospitalization
• Medical practitioner's 
prescription advising 
drugs, diagnostic tests or 
consultation
• Original test reports
• Original discharge card/ 
summary
• Indoor case papers
• Cancelled Cheque with 
Bank details
• Original bills from hospitals 
(final bill with break-up)
• Original bills from pharmacy, 
lab centers
• For accident cases, FIR copy 
(if applicable)
• Any other document as 
required by the company to 
assess the claim
How do I claim?
Reimbursement of Claims
$okfdoc$, 21),
  ('harassment', 'Anti-Harassment Policy', 'Conduct & Compliance', 'markdown', $okfdoc$# Anti-Harassment Policy

_Imported from the original policy document; HR can edit and reformat this in the portal._

Purpose 
The purpose of this document is to assure that Codezero2pi is taking all feasible steps to prevent 
harassment from occurring and to address conduct that does occur before it becomes severe or 
pervasive. 
Policy 
It is the policy of Codezero2pi to maintain a work environment that is free from harassment 
based on race, colour, religion, sex (whether or not of a sexual nature and including same -
gender harassment and gender identity harassment), disability (mental or physical), and sexual 
orientation and from ret aliatory harassment based on opposition to discrimination or 
participation in the discrimination complaint process. 
In addition, the policy states that no retaliation will be accepted against any employee for 
reporting harassment under this or any other policy or procedure, or for assisting in any inquiry 
about such a report. 
Scope 
The Policy is applicable to all Employees of Codezero2pi whilst they are on Workplace, 
doing work related activities and also any activities or event work related or otherwise, w hich 
may take place offsite and any place visited by the employee arising out of or during the course 
of employment. 
This Policy with immediate effect extends to all the Employees (defined hereinafter) of 
Codezero2pi and is deemed to be incorporated in the  service conditions of all the Employees. 
The policy shall also apply to customers, vendors, business colleagues or representatives from 
other organizations /establishments with whom the employee may connect or work within the 
course of official responsibility or work. 
 
 
Definitions 
 
Terms Definition 
Harassment It covers any form of unwanted and deliberate offensive behaviour. 
Nature can be physical or mental. Any significant emotional distress will 
also be considered as harassment. In the legal sense, it is behaviour which 
is found threatening or disturbing. 
 
 
Unwelcome verbal or physical conduct based on race, colour, religion, 
sex (w hether or not of a sexual nature and including same -gender 
harassment and gender identity harassment), disability (mental or 
physical), sexual orientation, or retaliation, constitutes harassment when: 
● The conduct is sufficiently severe or pervasive to crea te a hostile 
work environment; or 
● Less favourable treatment of a person by another or others in the 
workplace, which includes behaviour that intimidate, offends, 
degrades or humiliates an employee. 
Judiciary 
Committee: 
A group of employees who would be involved in investigation and 
decision making, while a case of harassment is being reported by any 
employee. 
The committee in Codezero2pi includes the 1 HR, and 2 other Head of 
Department. One more employee will be include d in the committee as 
per HR discretion. 
One of the Committee members has to be a female. 
Codezero2pi 
Premises or 
Workplace 
“Codezero2pi Premises or Workplace” shall include all land, offices, 
buildings, lodging, quarters, equipment, vehicles and parking areas under 
the control of Codezero2pi. Codezero2pi premises shall include such 
other work locations as the job site of a Cli ent office, including the 
transportation vehicles (e.g., airplanes, trains, busses, or automobiles) 
occupied during the travel to or from those locations while on Company 
business, or any location where Codezero2pi business is transacted 
Employee “Employee” means a person employed at a workplace for any work on 
regular, temporary, ad hoc or daily wage basis, either directly or through 
an agent, including a contractor, with or without the knowledge of the 
principal employer, whether for remuneration or not,  or working on a 
voluntary basis or otherwise, whether the terms of employment are 
express or implied and includes a co -worker, a contract worker, 
probationer, customers, visitors, trainee, apprentice, interns, or 
consultants or called by any other name; 
Complainant  An individual who alleges to have been subjected to any act of 
harassment by the Respondent at Codezero2pi workplace; 
 
 
Respondent  
“Respondent” shall mean a person or persons against whom the 
Complainant has made a complaint. 
 
 
Examples of Harassment at workplace 
The following behaviours would be examples of Sexual harassment: 
● Physical contact or requests for sexual favours; 
● Persistent following (stalking); 
● Suggestive looks implying a sexual interest; 
● Making sexually coloured remarks; or 
● Showing pornography or other offensive or derogatory pictures, cartoons, representations, 
graphics, pamphlets or sayings; or 
● Any other unwelcome physical, verbal or non-verbal conduct of a sexual nature; 
 
 
Some more examples of bullying or harassing behaviour could include: 
● Persistent verbal abuse or threats; or 
● Persistently disrupting an individual’s work, work space, 
● Equipment or interfering with their personal property. 
● Spreading malicious rumours; 
● Unfair treatment; 
● Regularly undermining a competent worker; 
● Denying someone’s training or promotion opportunities; 
● Circulating, displaying written or pictorial material that is offensive or belittling; 
● Jokes, derogatory or dismissive comments; 
● Gestures that are insulting or belittling; 
What is not Workplace Harassment 
It is important to bear in mind that there is a wide range of ambiguous behaviour that might 
offend some people, but not necessarily others. Examples might include: 
● Comments on clothing, compliments about improved appearance, and even unintentionally 
offensive jokes that most people might find reasonable. These types of behaviour would 
not normally be seen as harassment. 
● A social relationship welcomed by both individuals. 
● Friendly gestures among co-workers such as a pat on the back. 
 
 
● It is also important to note that, in the course of their 
work, supervisors have a responsibility to take difficult decisions, e.g., about moving 
people or changing work assignments. These decisions do not, in themselves, constitute 
harassment. 
● Work related stress in itself does not constitute harassment, but the accumulation of stress 
factors may increase the risk of harassment. 
● Also, a negative performance report, as such, is not harassment. Supervisors have a 
responsibility to give appropriate  feedback and to take appropriate corrective action. 
However, such feedback should be made in a reasonable and constructive manner and 
should not be used as retaliation. 
● Workplace conflict in itself does not constitute harassment but could turn into harass ment 
if no steps are taken to resolve the conflict. 
Expectations from Employee  
All Codezero2pi employees are responsible for implementing the anti -harassment policy and 
for cooperating fully in its enforcement. And to do so, every employee of the organiza tion is 
expected to follow the following points: 
● Employees must not engage in harassing conduct. 
● Employees subjected to harassment should promptly bring the matter to the attention of the 
committee. 
● Supervisors and other management officials must act promptly and effectively to correct 
any harassment that does occur. 
Expectations from Committee 
● The committee (on receiving the reports of harassment), shall be responsible for further 
inquiries into such  reports when necessary and must provide oversight, assistance and 
support the employee being harassed to assure compliance with this policy. The committee 
shall assure that the process is swift, thorough, impartial and appropriate to the allegation. 
Other parameters regarding the committee shall be:- 
● At least one-half of the total Members so nominated shall be women; 
● Members of committee shall hold office for such period, not exceeding three years, from 
the date of their nomination as may be specified at the time of appointment; 
● If any member of the committee, who is in employment of Codezero2pi, leaves the 
employment or is discharged, dismissed, terminated or suspended from his or her services, 
then she/he will automatically cease to be the member of the committee. 
● The CEO shall appoint another person as Member of committee in place of such Member 
within 30 (thirty) days from the date of cessation of his/her membership. 
The Committee members may be removed from the committee if: 
a. contravenes the provisions of above-mentioned expectations. 
 
 
 
 
b. has been convicted for an offence or an 
inquiry into an offence under any law for the time being in force is pending against 
him/her; or 
c. has been found guilty in any disciplinary proceedings or a disciplinary 
proceeding is pending against him/her; or 
d. has abused his/her position as to render his/her continuance in office prejudicial 
to the public interest. 
Complaint submission and Investigation Procedure 
If you believe you are the subject of harassment in violation of this policy, you should discuss 
the occurrence with any of the committee members with whom you feel comfortable discussing 
the problem. You should submit the complaint at hr@rapidinnovation.io.The complaint must 
be submitted within 15 (fifteen) days of alleged incident and in a series of inciden ts, within a 
period of 2 (Two) months from the date of the last incident. 
● In addition, employees who observe or are made aware of possible harassment in the 
workplace have an obligation to immediately report the incident to their immediate 
supervisor or any of the members of the committee. 
● When a supervisor/manager is notified of alleged harassment, he or she must notify the 
committee members immediately. 
● A complaint alleging harassment, whether written or oral, should include the specific 
nature of the incident, date and place of the incident, names of all parties involved, as well 
as a detailed report of all pertinent facts. 
● Complaints of harassment will be promptly and carefully investigated. 
● The employee should register the complaint to the committee bef ore making a police 
complaint. 
The process of investigation is as follow: 
1. All information will be maintained on a confidential basis to the greatest extent possible. 
2. As part of the formal investigation, on receipt of complaint, the Committee shall forward a 
copy of the same to the Respondent within 7 (seven) working days. The Respondent shall 
file his reply to the complaint along with his list of documents and witnesses at the earliest 
and in any case not later than 10 (ten) working days from the date of receipt of the copy of 
complaint from Committee. 
3. The complainant, respondent and any witnesses will be interviewed and asked for their 
account as to the events that have allegedly occurred. 
4. In order to maintain confidentiality, only the people involved in the investigation of a 
formal complaint are to be spoken to in regard to the matter. The complaint is only to be 
discussed with other parties on a need -to-know basis. 
5. The committee may, before initiating an inquiry and at the request of the complainant take 
steps to settle the matter between the complainant and the Respondent through conciliation, 
provided that no monetary compensation shall be made as the basis of conc iliation.  If the 
settlement is arrived during conciliation proceedings, the committee will record the 
 
 
settlement so arrived and provide copies of the 
suitable action as per the settlement, each to the complainant as well as the Respondent and 
also to the HR dept.  No further inquiry shall be conducted where a settlement has been 
arrived post conciliation. 
6. If no settlement is arrived, Investigations would be proceed further which will include 
discussion with all relevant persons, including the accused and other potential witnesses. 
7. If after the investigation, the committee members are satisfied on the balance of 
probabilities that the complaint is upheld then the Respondent is liable to disciplinary 
action. 
8. Appropriate remedial action will be taken in all ca ses where harassment is found to have 
occurred. Such remedial action may include termination of employment. 
9. Codezero2pi prohibits any form of revenge against any employee for making a good faith 
complaint under this policy or for assisting in a complaint investigation.  
10. However, if, after investigating any complaint of harassment or unlawful discrimination, 
the Commission determines that an employee has provided false information regarding the 
complaint, disciplinary action up to and including termination m ay be taken against the 
individual who made the complaint or who gave the false information. 
11. Note: Most cases of sexual harassment occur in private, so there may not be any eye -
witness. The committee will have to come to a conclusion about the complaint wi thout 
proof or evidence of this kind. It will rely on circumstantial evidence and the written 
submissions and oral testimonies of the Aggrieved Woman, the Respondent and witnesses 
if any as well as any documentary evidence. This inquiry is not a criminal investigation or 
a proceeding in a court of law – a strong probability, rather than ‘proof beyond reasonable 
doubt’, is enough to take a decision on the Complaint. 
Guidelines for Compliant 
While you file a complaint against anyone, keep a note of events and  any supporting 
documentation including: 
• What happened? 
• Who was involved? 
• When did the incident took place? 
• Where did the incident took place? 
• How did you react? 
• Was this the first time this has occurred or has it happened earlier also? 
• Did anyone else see this, or any previous incident? 
• Is there any other physical evidence or documentation of the incident? 
 
 
 
Withdrawal of Complaint 
If at any stage, after the filing of a complaint and during the pendency of inquiry proceedings, 
the Complainant wishes to withdraw the complaint, then he/she shall have the right to withdraw 
the complaint and the Committee shall permit him/her to withdraw  the complaint and if an 
 
 
inquiry has commenced, then the committee shall 
discontinue the inquiry without giving any findings or conclusions on merit. However, the 
committee must ascertain the reasons for withdrawal of the Complaint, record the same in 
writing and get it countersigned by the Complainant.
$okfdoc$, 30),
  ('email-slack', 'Email & Slack Usage Policy', 'Conduct & Compliance', 'markdown', $okfdoc$# Email & Slack Usage Policy

_Imported from the original policy document; HR can edit and reformat this in the portal._

Usage of E-mail and Slack 
Purpose 
The document outlines the details of the usage of E-mail & Slack respectively. 
 
Scope 
These uses of Email and Slack applies to everyone who is associated with Rapid Innovation 
 
E-mail and Slack Usage 
 
Below are a few of the sample scenarios which distinguish Emails and Slack communication based upon following 
parameters. 
        Formal/Informal 
        Client/ Internal 
        Individual/Team 
        Inter departmental/ external to department 
  
Sample Email Scenarios Sample Slack Scenarios 
Any communication being sent to Client Chat with client in case of non-responsiveness 
over emails 
Approval of documents (PSD/TRD etc) Minor clarification from client 
Approval of requirements Internal connect needed by team/individual 
Company wide communication from HR related 
to Payroll/Fun Activity/Employee Induction etc 
Sharing of project notes by/among 
individual/team 
Project go-live emails Anything non critical item which is informal 
Publication of Process related changes Reiterating or reminder on message sent over 
email 
Communication impacting all departments 1:1 Reminder and follow-up 
Any type of approval (internal) Informal chat 
Any confidential/ sensitive topic Birthday wishes 
  
With reference to the above, has to be followed from immediate effect i.e. by Monday 1st August 2022, we request all 
the employees to use only E-mail as a formal mode of communication. In case of any clarifications/concerns please feel 
free to reach out to HR.  
 
Important Points be noted: 
 
1. Anyone who receives mail has to acknowledge it within same working day & need to keep the email sender 
updated about status to the mail with-in 24hrs of receiving the mail.  
2. Reverts on slack messages are supposed to be responded back within the same day. 
3. Usage of Slack should be limited. Anything important should always be sent via email and slack can be used as a 
reminder tool or for quick updates. 
4. Only Official mail ids should be used only for official communication.
$okfdoc$, 31),
  ('camera', 'Camera Usage Policy', 'Conduct & Compliance', 'markdown', $okfdoc$# Camera Usage Policy

_Imported from the original policy document; HR can edit and reformat this in the portal._

Version Author Publishedon Remarks
1.1 HRD 7thApril 2023 First Release.ExplainstheguidelinesforUsingcamera
UsageofCamera
Purpose:
This policy establishes guidelines on the use of the Camera during meetings(internal/external)/sessions/trainingsat RapidInnovation
Scope:
This policy applies to everyone associated with Rapid Innovation. As we are working in aremote environment, in order to have clear communication and understanding video calls aremandatory.Another objective of camera usage is to have teambonding and effective collaboration, whichcan only be achieved via personal connect over videocalls. Webelievethat employeesactivelyparticipate when they are on video and other participants can connect effectively. This not onlyincreasesproductivitybut addsapersonal touchtothemeeting.
Guidelines:
● Using a camera during meetings (internal/external)/sessions/training is mandatory foreveryoneat Rapid.● If any employee regularly fails to use a camera during meetings/sessions/training, strictactioncanbetakenagainst them.
Generaletiquetteswhileusingcamera:
● Mute your computer or microphone when you are not speaking to avoid backgroundnoise.● Ensurethat therearenodistractionsintheroom, suchasother peopleor loudnoises.

● Dressappropriatelyfor thevideocall,takingintoconsiderationtheaudience.● Maintainaprofessional andappropriatebackgroundduringthevideocall.● Jointhecall usingyour official email ID.● Make sure you all have a stable and reliable internet connection with aminimumspeedof 20Mbps.
Pleasefeel freetoreachtheHRDepartment incaseof anyqueriesor concerns.
$okfdoc$, 32),
  ('laptop-asset', 'Laptop & Asset Policy', 'IT & Assets', 'markdown', $okfdoc$# Laptop & Asset Policy

_Imported from the original policy document; HR can edit and reformat this in the portal._

ASSET POLICY 
 
Table of Content 
 
 
 
 
Table of Content  2 
Purpose  3 
Scope  3 
Guidelines  3 
Eligibility and process of availing Desktop/Laptop/Asset  3 
Desktop/Laptop/Asset usage  3 
Desktop/Laptops/Assets at your remote workplace  4 
Reporting a Theft  4 
Ending employment with the Company  5 
Submission of Desktop/Laptop during long term absence  5 
Reporting damage  5 
General Expectations from Employees 5  
Do’s & Don’ts  6 
Declaration  6 
The declaration form is enclosed below.  6 
 
Annexure I 
 
Purpose 
The document outlines the provisions pertaining to usage of laptops/assets provided by Rapid 
Innovation to its employees. 
 
Scope 
This policy applies to all employees who use Company owned laptops/assets. Every 
employee entitled with the Company-owned laptop/asset is responsible for the security of 
that laptop and the accessories, regardless of whether the laptop is used, at one's place of 
residence, or in any other location such as a hotel, conference room, while travelling. 
 
Guidelines 
 
Eligibility and process of availing Laptop 
● The laptop would be issued by IT/HR Dept whoever falls in the Laptop Policy. Only 
confirmed employees are eligible for companies’ laptop/asset 
● A declaration form has to be signed by the employee before getting the laptop and 
related accessories. 
● The accessories would include: Laptop adapter, phone, bag, mouse and 
headphone (if required). 
● The laptops would be issued to an employee post confirming the requirement from 
the respective project head & group head, with the standard configuration by IT/HR 
dept. 
● Those employees who are using the company's laptop/asset are informed that, we 
will not be withholding any amount in name of Laptop/asset instalments but if 
there is any damage/theft of the laptop/asset then the company will be bearing the 
50% cost of it and 50% the employee needs to bear. 
 
 
Laptop /Asset usage 
 
All employees who have issued the laptop must use the laptop only for official purpose in the 
course of their rightful discharge of their duties and not be used for generating, transmitting, 
corresponding anything that is unlawful or abusive. 
 
Laptop/Asset usage 
When an employee is using the laptop at their homes or any other place out of their homes, 
he/she is expected to keep the laptop in hand or sight, or in a secure location, at all times. It is 
the responsibility of the employee to handle the laptop/asset carefully. During the period, 
when the employee has the laptop/asset with them the same should not be misused for the 
purpose of transferring the data on to other storage devices. 
 
Limited access to Software 
● Rapid Innovation reserves the right to provide access to any software for the 
laptops/asset issued to our employees. The access to such software’s would be 
limited as per the roles and business requirements. For instance: MS office would be 
provided to only those employees whose role requires such software. Other such 
software’s could be adobe illustrator, Photoshop etc 
● The paid software should not be imitated on other devices neither copied or on pen 
drive nor shall be uninstalled /reinstalled without prior permission from IT/HR Team. 
● The employees are also expected to be responsible for not installing any unauthorized 
software’s 
 
Storage limit 
Rapid Innovation would provide a standard configuration laptop/asset to all our employees 
based on the job requirement. The storage device can be extended further on required 
approvals from Group Heads. 
 
Reporting a Theft 
 
a)  Your remote working location: 
If a Company-owned laptop/asset is stolen or lost while working remotely, or while taken the 
laptop/asset outside your remote work space, the employee must immediately inform IT/HR 
Dept of Rapid Innovation, along with details like time, date, location and any other details 
that you feel is important and file the FIR in the nearest police station as soon as possible, 
without a FIR we won’t consider the laptop to be stolen and we won’t be able to help you in 
that case and the entire laptop cost you will have to bare then. 
 
 
Ending employment with the Company 
The employee must return the laptop/asset and related accessories (issued during the tenure 
with Rapid Innovation) to the Company before leaving the organization. As we are working 
remotely the laptop has to be couriered back to us post receiving it we will be releasing your 
Full an Final Settlement amount. 
Any assigned accessories not submitted may lead to recovery in the full and final settlement. 
 
Submission of Laptop/asset during long term absence 
During any long-term absence i.e., marriage leave, etc the laptop must be submitted to the 
IT/HR Dept., unless the employee gets an approval from the management and the same must 
be informed to HR and the Reporting Manager. 
 
Reporting damage 
All the employees are expected to take care of the laptop/asset and related belongings. Any 
damage (due to any reason) should be reported immediately to the IT/HR team. 
 
Penalty 
Employee will have to bear 50% of the cost of wear and tear due to accident/theft. Rest 
these things can be discussed case to case basis 
 
 
General Expectations from Employees 
● The Employee is responsible for maintaining monthly backup files of their 
Laptop/asset as an added precaution against data loss. 
● Laptop shall be the property of the Company at all times and the Employee will not 
have any right or interest in the said asset except using such asset during the 
employment or for such duration as may be decided by the Company 
● Employee shall maintain confidentiality, at all times, with respect to all the data and 
information relating to the Company. 
● Personal Device like any USB cable, pen-drive, hard disk, RAM etc is prohibited and 
must be used only with prior approval. 
● The Laptop/asset strictly should be used only for official purpose and not for any 
personal usage. 
Do’s & Don’ts 
Do’s 
● Maintain all your files in synchronized manner; avoid keeping personal files in 
companies’ laptops/asset. 
● Keep the laptop/asset away from dust. 
● Handle the laptop carefully; do not hold it by screen or any wrong manager. 
● Shut down the laptop once you are logging off from work and also while putting the 
laptop inside the bag else it may get damaged due to heat recirculation 
 
Don'ts 
● Do not keep any beverages on or near the laptop/asset. 
● Do not clean your laptop/asset with water. 
● Do not keep your laptop/asset on cloth. 
● Do not format the Laptop/asset without permission. 
● Never let your laptop/asset battery run out. 
● Don't rest your laptop/asset on uneven, unsteady or yielding surfaces. 
● Don’t place your laptop/asset near other equipment that emits heat/magnetic effect. 
● Do not maintain the personal data on official laptop/asset. 
● Do not take the laptop or its accessories for repair to any external agency or vendor 
at any point of time, without prior approval from the IT/Admin Dept and 
Management. 
● Do not purchase any hardware for laptop from outside without prior permission from 
IT/Admin dept, and if done, a detailed bill is required to be submitted. 
● Don't use any other brand adapter with your Laptop/asset. 
● Don’t use it for personal usage. 
 
 
Declaration 
All our employees shall sign a standard declaration form at the time of issuance of laptop/asset 
and related accessories, as to the acceptance of the policy. 
 
 
 
 
Annexure I 
 
DECLARATION CUM UNDERTAKING 
 
 
Name: 
 
Emp ID: 
 
Department: 
 
Designation: 
 
Residence Address: 
 
Contact no.: 
 
Laptop Model/Serial No.: 
 
OS Details: 
To use exclusively for the purpose of conducting the Company’s business and undertake the following: 
● The laptop issued is for solely official purpose 
● I shall be fully accountable for theft, loss or damage of the property and related accessories, 
and any data stored in the device 
● I undertake to exercise adequate care to maintain the Laptop including accessories, such as 
battery charger, bag, headphones, network ports etc. 
● I undertake to respect all the copyrights and licenses to software and other on/off -line 
information and will not upload, download or copy software or other material (apart from 
approved software/information) without the prior consent of the Management.  
● In case of any malfunction, I am required to report the same to the IT/ HR Department 
● I would not take the laptop or its accessories for repair to any external agency or vendor at 
any point of time 
● The laptop should be returned to the IT/HR Department in case of leaving the organization 
or if I do not intend to use it for any reason. 
● I undertake not to hand over the Laptop to any other person without a written approval of 
Management. 
 
Date 
 
Location 
New Laptop - 
Old Laptop - 
 
 
 
Employee Signature: IT/ HR Signature:
$okfdoc$, 40),
  ('purchase-process', 'Procurement / Purchase Process', 'Finance & Procurement', 'markdown', $okfdoc$# Procurement / Purchase Process

_Imported from the original policy document; HR can edit and reformat this in the portal._

Codezero2pi Solutions (OPC) Pvt. Ltd.  Procurement process 
1 
 
 
Procedure to be followed for any purchase which is to be made in the name of the 
company: 
 
1) Staff to raise the requirement with relevant department head in case of a technical 
request. (For eg: software for team use).  In case there is a general request ( For eg: 
purchase of accessories), staff will contact HR department directly. 
2) Once HOD agrees, he / she will have to fill the attached form with all the details required 
to generate the request. In case of general requisition, HR will get the form filled from the 
staff and send to relevant HOD. 
3) Once the form is filled, HOD to send the same to Finance department keeping relevant 
CXO in cc. In case of general requisition, HOD will send the same with CXO and HR in cc. 
4) Finance will confirm the same make the purchase and intimate relevant HOD on the same 
mail thread along with any detail required like log in credentials etc. 
 
*There is an approval at every step and request can be rejected at any stage. So none of the approval to be 
considered as final. 
  
Codezero2pi Solutions (OPC) Pvt. Ltd.  Procurement process 
2 
 
 
Figure 1 Technical requisition 
Software / technical 
expense requirement 
generated with HOD.
HOD to fill out 
attached template 
with all the 
information required 
make the purchase.
HOD to send the file 
to Finance keeping 
relevant CXO in cc. 
Finance will confirm 
and make the 
payment.
Codezero2pi Solutions (OPC) Pvt. Ltd.  Procurement process 
3 
 
 
Figure 2 General requisition 
  
General expense 
requirement 
generated with 
HR.
Staff to fill out 
attached 
template with all 
the information 
required make 
the purchase.
HR to send the 
form filled to 
HOD.
HOD to send the 
same to Finance 
keeping CXO and 
HR in cc.
Codezero2pi Solutions (OPC) Pvt. Ltd.  Procurement process 
4 
 
 
Annexures: 
Purchase requisition CZ2P.xlsx
 
 
HR email: hr@rapidinnovation.io 
Finance email: krishna@rapidinnovation.dev 
CXO and HOD email as applicable.
$okfdoc$, 50),
  ('pip', 'Performance Improvement Plan (PIP) Policy', 'Performance', 'markdown', $okfdoc$# Performance Improvement Plan (PIP) Policy

_Imported from the original policy document; HR can edit and reformat this in the portal._

PIPPolicy
Purpose:Aperformanceimprovement plan(PIP), alsoknownasaperformanceactionplan, isatool togiveanemployeewithperformancedeficienciestheopportunitytosucceed.
Scope:Applicableonthoseemployeeswhoarenot performingwell
Definition:Aperformanceimprovement plan(PIP) isadocument that aimstohelpemployeeswhoarenot meetingjobperformancegoals.
WhentoputemployeeunderPIP
Anemployeehastoput intoPIPwhenhe/sheisnot performingasper thecompanyrequirementsandexpectations. BeforeputtingintoPIP, managersneedtogiveawrittenemailwarningtotheemployeethat his/her performanceisnot uptothemarkkeepingHRinloop. 15dayspost that warningstill if weseenoimprovementsthentheemployeehastobeput underPerformanceImprovement Plan.
Process
Step1: Plan
Use the template to prepare a performance improvement plan for your employee. Begin byclearly identifying the specific area or areas in which the employee needs to improve theirperformance. Then we need to drop employees under PIP email keeping HR in the loop, onwhat all areas they need to work on. We need to define a timeline as well for which employeeneeds to undergo PIP. Aminimumof 15 days and a Maximumof 30days, isthetimelineonwhichwecanput anyemployeeunder PIP.
Step2: Meetwithyouremployee
Explain what your employee needs to do to improve their performance and how they can dothis, along withwhat support you’ll providetothem(eg. training). Alsoexplaintothemwhat theirresponsibilities are, andwhat your responsibilitiesare. Giveyour employeeareasonabletimetoimprove their performance and set a date or dates for further review. Finally, explain what willhappen if your employee’s performance doesn’t improve. Both you and your employee shouldsignandkeepacopyof theplan.
Step3: Monitor
Monitor your employee’s performance while the plan is in place. Regularly check in with youremployeeover that periodtodiscusstheir progress.

Step4: Review
Meet at the times set out in the plan to review your employee’s performance. Before thesemeetings, both you and your employee should assess their performance. After thesemeetings,you should update theplantomakesureit stayscurrent (eg. toexplainwhat your employeestillneedstoimprove, andanyfurther support that you’ll provide).
Step5: FinalUpdate
After closing monitoring the performance of the employee under PIPif thereisnoimprovementhe/she has to be terminated with immediate effect no notice period will be given under anycircumstances. This can be done in the middle of the PIPperiod as well if no improvement isseen.
Kindly note- In case the employee is terminated due to non performance under the plan thenhe/she will not be eligible for any leave enchasment or any other benefit on exit from thecompany.
$okfdoc$, 60),
  ('goa-workation', 'Goa Workation Guidelines', 'Culture & Events', 'markdown', $okfdoc$# Goa Workation Guidelines

_Imported from the original policy document; HR can edit and reformat this in the portal._

Goa  Workation  Guidelines  and  Information      
Purpose The  document  outlines  the  details  of  our  Goa  office.    
Scope This  policy  applies  to  all  employees/Interns  (along  with  their  family)  who  are  visiting  our  Goa  
office.
 
Every
 
employee
 
visiting
 
the
 
office
 
is
 
obliged
 
to
 
follow
 
the
 
guidelines,
 
and
 
should
 
be
 
aware
 
about
 
each
 
and
 
every
 
thing
 
for
 
eg.
 
Addresses,
 
Phone
 
Numbers,
 
Do’s
 
&
 
Don'ts,
 
Etc.
   
Definition
 
of
 
Family:
 
Family  for  Married  Employee/Intern  Employee/Interns  +  Spouse  +  Two  Children  
Family  for  Unmarried  Employee/Intern  Employee/Interns  +  Single  Parent  
  
 Approval  Process:   In  order  to  plan  your  trip  to  Goa  office  you  need  to  follow  a  process:  1.  Drop  a  mail  to  your  Project  Manager  &  Group  head  regarding  your  trip  to  Goa.  The  
HR
 
should
 
be
 
lopped
 
into
 
that
 
mail.
 2.  Once  you  get  an  approval  from  the  Group  heads  and  Project  Manager  inform  the  HR  
before
 
making
 
any
 
bookings.
 
Once
 
the
 
HR
 
confirms
 
that
 
the
 
accommodation
 
is
 
available
 
you
 
can
 
make
 
the
 
bookings
 
accordingly.
     
Guidelines   Do’s  1.  Take  care  of  your  belongings  2.  Proper  care  of  all  the  assets  should  be  taken.  3.  Have  fun  for  sure.  4.  Plan  out  your  work  timings  if  you  want  to  explore  around.  5.  Everyone  is  requested  to  travel  on  Weekends  or  after  working  hours.  6.  If  you  are  travelling  on  weekdays,  take  prior  permission  from  your  manager  by  
dropping
 
mail
 
&
 
keeping
 
HR
 
in
 
loop
 
&
 
apply
 
leave
 
via
 
Razorpay.
    
 

 
Don’ts  1.   No  consumption  of  alcohol  during  working  hours.  2.  Don’t  damage  any  asset  at  the  property;  everything  should  be  handled  with  care.  If  
anything
 
is
 
damaged
 
in
 
that
 
case
 
you
 
will
 
be
 
liable
 
to
 
pay
 
a
 
fine
 
on
 
the
 
damaged
 
item/product.
 3.  A  person  has  to  work  according  to  the  work  hours  of  the  office.  If  they  plan  to  explore  
around
 
they
 
need
 
to
 
manage
 
their
 
work
 
timings
 
accordingly.
 4.  Productivity  shouldn’t  be  affected  during  your  workcation.  5.  Keep  the  premises  clean  6.  Try  not  to  travel  during  weekdays  as  that  would  hamper  the  work.   NOTE:  Kindly  note  that  we  will  accommodate  only  on  a  first-come  first-serve  basis.  
Only
 
accommodation
 
will
 
be
 
company-sponsored,
 
food
 
and
 
travel
 
have
 
to
 
be
 
taken
 
care
 
of
 
by
 
the
 
employees
 
themselves
 
  Mentioning  some  Important  details  that  everybody  should  know  while  visiting  the  Goa  
office.
  Address:-  Hotel  North  39,  Junas  Wada,  near  River  Bridge,  Mandrem,  Goa  403527  Contact  Details:-  Rapid  Innovation  Admin/Manager  at  North39  hotel  -  Armond  -    
9823262663
 
HR
 
Department
 
-
 
Aarushi
 
-
 
7300616246
     
WiFi  Names  and  Passwords  at  the  premises  
  
Floors  Wifi  Name  Password  
Ground  Floor  NORTH39GR2  north39@111  
First  Floor  NORTH39  north390578  
Second  Floor  NORTH39  2ND2  north39@333  
Office  RI  Office-2GHz  RI  Office-5GHz  
rioffice@39  
     Travel  :   Can  be  contacted  for  Airport  pick-up  and  drop.  1.  Goamiles  App  (Goa  Cab  booking  App)  (Check  if  Driver  has  accepted  your  booking  
else
 
book
 
a
 
local
 
Cab)
 2.  Local  Cab  Service  near  North39  hotel  -  7875911799  /  8408085841  (Airport  to  
North39
 
hotel
 
OR
 
North39
 
to
 
Airport–
 
Approx
 
Rs
 
1800)
 3.  Local  Cab  Service  near  North39  hotel  -  9545114931  
 

 
(Airport  to  North39  hotel  OR  North39  to  Airport–  Approx  Rs  1800)
$okfdoc$, 70),
  ('techtalk', 'TechTalk — Knowledge Sharing', 'Culture & Events', 'markdown', $okfdoc$# TechTalk — Knowledge Sharing

_Imported from the original policy document; HR can edit and reformat this in the portal._

TechTalk - Knowledge Sharing Sessions
As we were growing in numbers, along with working on more exciting projects, we realized thatwe should create an opportunity for our knowledge and new skills to be shared.We share knowledge differently while working on projects, in discussions, meetingsetc, but now we will organizeTech Talkin our officeas we have a need to gather together andtalk about Technical or Non- Technical things we do or currently work on.
Objective
This is an opportunity for us to share knowledge and exchange ideas.
When and Why
When - Every last Friday of the month, we will organize a tech talk via googlemeet enjoying anhour of exploring specific subjects our employees choose to present and explain.
Why  - Having the right channel to communicate and share ideas is crucial, but our wish to shareand acquire new knowledge is certainly recognized as the most important factor.
We as a family have an opportunity to gather together and listen to our employees' experienceand opinion, it can only be beneficial and a great advantage to everyone.
So, Why not use this Opportunity ????
Process
● Every last friday of the month we will organize this session.● Everytime we will have a new presenter from our office talking on any Topic of his/herchoice. It can be technical or non-technical● Presenter must prepare PPT for the topic he is going to present (Present can take helpfrom anyone he/she wants)● On the day of TechTalk we all will sit together and the presenter will present.

● After the session, we will share feedback forms with everyone so that each one of us asa presenter will get a chance to work on our shortcomings by the feedback we willreceive.
Each person is the wisdom and knowledge database, but only when shared – knowledge is ofvalue to others. The somewhat magic ingredient is trust.
Trust builds strong ties and empowers knowledge sharing.
So, Guys start digging into your expertise and be ready to share with us.
First session will be on 28th Jan 2022.
With Best RegardsHR Department
$okfdoc$, 71),
  ('confluence-guide', 'Confluence Employee Guide', 'Tools & How-to', 'markdown', $okfdoc$# Confluence Employee Guide

_Imported from the original policy document; HR can edit and reformat this in the portal._

Confluence Employee Guide
About Confluence
Create, collaborate, and organise all your work in one place. Confluence is a teamworkspace where knowledge and collaboration meet. Dynamic pages give your team a placeto create, capture, and collaborate on any project or idea. Spaces help your team structure,organise, and share work, so every team member has visibility into institutional knowledgeand access to the information they need to do their best work.
Confluence is for teams of any size and type, from those with mission-critical, high-stakesprojects that need rigour behind their practices, to those that are looking for a space to buildteam culture and engage with one another in a more open and authentic way.
Equipped with Confluence, your team can make quick decisions, gain alignment, andaccomplish more together.
Key terms
Page
Your content lives inpages– living documents youcreate on your Confluence site. You cancreate pages for almost anything, from project plans to meeting notes, troubleshootingguides, policies, and more. Confluence comes bundled withtemplatesto help you makebeautiful pages for almost any kind of content. If you can’t find a template for the type ofcontent you want to create, you can always start with a blank page.
Space
Pages are stored inspaces– workspaces where youcan collaborate on work and keep allyour content organised. It’s best to group related content together in the same space, butyou can create as many or as few spaces as your team needs. For example, one marketingteam might keep all of its work in one space, with a page for each campaign, while anothermight set up a separate space for every single campaign. Each space comes with anoverview (or homepage) and a blog, so it’s easy to share updates and announcements withyour whole team.
Page tree
Organise space content with a hierarchical page treethat makes finding work quick andeasy. Nest pages under related spaces and pages to organise pages in just about any way.

Getting Familiar with Terminology
1. Dashboard
The dashboard is the landing page that a logged in user sees after successful login. Thedashboard gives a quick snapshot of the recent updates by the team along with the recentupdates done by the user himself.
Along with the updates, the dashboard also shows the Spaces that the user is a member of.We will discuss more spaces in the next section. The sidebar containing updates and spacedetails is collapsible to optimise the viewing experience.
Below is an example of the Confluence Dashboard.
2. Learn about spaces and pages
Your Confluence site is organized intospaces. Spacesare collections of related pages thatyou and other people in your team or organization work on together. Most organizations usea mix ofteam spaces,software project spaces,documentationspaces, andknowledgebase spaces:
● Useteam spacesto encourage team members to worktogether toward large-scalegoals and OKRs.● Usesoftware project spacesto keep track of individual initiatives and projects.● Usedocumentation spacesto create and organize technical documentation foryour products and services, so it’s easy for anyone to use.● Useknowledge base spacesto store and surface answersto common questions,such as policy clarifications and IT solutions.● Use yourpersonal spaceas a sandbox to organise yournotes, keep track ofpersonal OKRs and goals, and draft proposals for projects before they make it to the

roadmap. Connect with your team by writing blog posts to introduce yourself or sharewhat you’re working on.
Below is an example of the spaces being created based on the differentorganisational units.
Space directory contains a list of all spaces that are created by confluence. You can browsethe spaces based on the space type – site, personal or my spaces. My spaces refer to thesites created by the logged in user himself and can be either site or personal space.
Below is an example of the space directory.
Confluence permits the creation of two spaces- site spaces and personal spaces. Below is acomparison of these space kinds:

Characteristic Site Spaces Personal space
Purpose Collaboration Personal work space
Accessible by - All Confluence users
- Access can be restricted based onGroups of users (similar to JIRA)
- Creator of space if sitemarked as private
- All Confluence users, ifspace is made public
Listed in Spacedirectory
yes No, accessible underpersonal profile of thecreator
Space Sidebar
The space sidebar is a collapsible menu on the space and pages and is used to navigatedifferent pages. The pages are shown in the form of a hierarchical tree structure.

Create functionality
Create functionality is used to create new pages within any chosen spaces in the desiredhierarchical order. We will discuss this functionality in more detail in the next section.
This image below pretty much summarises the main functionalities that you would beusing as a confluence user:
How to create and manage your own space and pages
In this section, we will discuss how to create and manage your own space and pages fromscratch.Once you know what kinds of spaces your organization will need, it’s time to createyour first space.
1. Go to your Confluence site.2. From the home screen, selectCreate Space.3. Select the type of space you’d like to create.4. Fill in theSpace name,Space key, and other details.5. Setpermissionsfor your space.6. SelectCreate.
Step #1: Creating your space with examples

Now choose the kind of space you want to create
Now fill in the required information in the next step. You will be required to enter spacename, a space key, and other mandatory or optional field depending on the kind of spaceyou chose.
The space key is a unique key used in the space URL and is auto-generated when usertypes in Space name, but you can change it if required.
Congratulations, you just successfully created your first Confluence space!!
Now let’s move on to creating some pages and content to share on this newly createdspace.
Step #2: Creating new pages with examples
You have the option to create a blank new page or chose from available templates.  The veryfirst page will be created as the Parent page. Subsequent pages can be created under thisparent page or as separate page depending on how you want to structure your space.
● Creating a blank page

● Creating page from available templates
Depending on the template chosen, you would be required to perform some additional stepslike entering the page name, etc. I chose the Retrospective meeting template and was askedto enter the Title and Participants.

The new page will be created and you can edit and fill in the required information.
3. Customize your space overview
Each space comes with anOverviewthat you can useto tell team members and otherstakeholders all about the purpose of your space and what they will find in it. If you createdyour space from a space template, your overview will come with built-in features to help youmake the most of your space. Even so, you may find adding your own touch lets you turnyour overview into the perfect hub for everything your team needs.
To customise your overview, select the pencil icon and edit the overview just like you wouldany other page.
Try these tricks to make your overview pop:
● Upload a banner or logo to help people identify your space at a glance● Describe your team’s mission and goals and add links to key pages● Add a table of contents, team calendar, or roadmap.

4. Organise your content
Now that you’ve created your first space, it’s time to get organised. The goal is to make yourspace easy to navigate so team members and other stakeholders can find the contentthey’re looking for quickly.
Use parent pages to group similar content
In Confluence, you can nest pages underneath other pages, creating a hierarchy of contentin each space. This hierarchy is reflected in the page tree, which appears in the spacesidebar to the left of the active page.
To use the page tree to your advantage, create a page for each task or project your team isinvolved with and nest related child pages underneath it. For example, if your team conductsretrospectives every 2 weeks, you might have a top-level page called “Retrospectives” with apage for each retrospective you’ve conducted nested beneath it.
The example below shows how one Atlassian team utilizes this strategy to organize theirspace:
Create shortcuts for important pages
Confluence lets you create uniquespace shortcuts– links that are pinned to the spacesidebar, above the page tree – for every space in your site. Use these to highlight importantcontent so it’s easy to find.
To create your first space shortcut, navigate to your space and select+ Add shortcut in thesidebar.

Label pages and attachments
Labels make it a breeze to identify related pages and attachments, so team members andother stakeholders can find what they’re looking for.
1. Open the page in Confluence.2. Select the label icon () in the bottom right.*3. Enter the name of the label you’d like to apply. If a label with that name alreadyexists, it will appear in the autosuggest menu.4. SelectAddto apply the label.5. SelectCloseto exit the dialog.
*If you’re editing the page instead of viewing it, select the more actions menu (•••) in the topright, then selectAdd labels.
Give labels transparent and meaningful names.
Keep content organised
Set aside time to review the content in your space,deleteorarchiveobsolete content, andmove pages aroundto maintain the structure you want.
5. Navigate Confluence
With Confluence Cloud, you can bring all your information, documents, and projects into oneorganized hub for teams to easily navigate, search, and discover information, fast. No moresifting through emails and shared folders for that one document you need; everything lives inConfluence.
This guide will teach you how to navigate Confluence Cloud so you can find the informationyou need to do your best work.
Site navigation
Use the site navigation menu to find people, pages, and apps no matter where you are inyour Confluence Cloud site:
● Tap Home or the Confluence logo to return to your Confluence Cloud dashboard. TapRecent to see a list of pages and blog posts you’ve visited or worked on, plus draftsand starred content.● Tap Spaces to move between your starred or recently visited spaces, or to accessthe space directory.● Tap People to visit the people directory, where you can find information about thepeople you work with and create teams.● Tap Apps to see a list of the apps that are installed on your site or visit an app’sdashboard.● Tap Create to create a new page from anywhere on your site.

Navigating within a space
Use thespace sidebarto navigate within a space.The space sidebar appears to the left ofthe page you’re viewing. It has three parts, each specific to the space you’re in.
The name of the space appears at the top of the space sidebar, followed by links to thespace overview, blog, and space settings, plus dashboards for any apps you have installed.
Below these items, you’ll find yourspace shortcuts.These are links to important pages orwebsites that people who use the space need to be able to find easily. You can addshortcuts to pages in the space, in other spaces, or even on external websites.

Finally, there’s the content in the space.Pagesareorganised and displayed hierarchically inthe page tree. To see the children of any page in the space, tap > next to the page name.Confluence Cloud automatically displays the children of the page you’re viewing.
$okfdoc$, 80),
  ('spoc-list', 'SPOC List (Points of Contact)', 'Directory', 'markdown', $okfdoc$# SPOC List (Points of Contact)

_Imported from the original policy document; HR can edit and reformat this in the portal._

SPOC List
Purpose: The purpose of this list is that each and every employee should be aware about whom he/she needs to connect when facing challenges
Definition: SPOC List "Single Point Of Contact" refers to a single person or team within a company who is designated as the point of contact for all incoming communications. The aim of having this list is usually to avoid miscommunication between HOD/HR and employees.
Scope: This list is helpful to everyone associated with Rapid Innovation.
HOD’s  List
Please Note - Contact all Group heads on E-mail & slack first.

HR  SPOC List
Other issues:	Please drop a mail at teamhr@rapidinnovation.dev, someone will contact you directly from the HR  team.
HOD Name | Designation | Email ID | Contact No
Abhijeet Sonaje |  Director of Engineering | Abhijeet@rapidinnovation.dev | 9579534174
Shailesh Kala | Director of Engineering | sk@rapidinnovation.dev | 7895744872
Hitesh Goyal | Director of Engineering | hitesh@rapidinnovation.dev | -
Deepak Pal | VP of Operations | deepakpal@rapidinnovation.dev | 9899487371
Name | Designation | Email ID | Contact No.
Aarushi
Sharma | Assistant Manager | aarushi@rapidinnovation.dev | 7300616246
Deepti 
Sharma | HR Generalist | deepti@rapidinnovation.dev | 9315659478
$okfdoc$, 90)
ON CONFLICT (slug) DO NOTHING;

DROP TABLE IF EXISTS okf_document;
