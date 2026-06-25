<?xml version="1.0" encoding="UTF-8"?>
<tileset version="1.10" tiledversion="1.12.2" name="bat" tilewidth="32" tileheight="32" tilecount="440" columns="11">
 <properties>
  <property name="airborne" type="bool" value="true"/>
  <property name="hitbox_height" type="float" value="1"/>
  <property name="hitbox_width" type="float" value="1"/>
 </properties>
 <image source="bat.png" width="352" height="1280"/>
 <tile id="0">
  <properties>
   <property name="action" value="attack"/>
   <property name="dir" type="int" value="0"/>
  </properties>
  <animation>
   <frame tileid="0" duration="100"/>
   <frame tileid="1" duration="100"/>
   <frame tileid="2" duration="100"/>
   <frame tileid="3" duration="100"/>
   <frame tileid="4" duration="100"/>
   <frame tileid="5" duration="100"/>
   <frame tileid="6" duration="100"/>
   <frame tileid="7" duration="100"/>
  </animation>
 </tile>
 <tile id="4">
  <properties>
   <property name="apex" type="bool" value="true"/>
   <property name="sfx" value="bite01"/>
  </properties>
 </tile>
 <tile id="11">
  <properties>
   <property name="action" value="attack"/>
   <property name="dir" type="int" value="1"/>
  </properties>
  <animation>
   <frame tileid="11" duration="100"/>
   <frame tileid="12" duration="100"/>
   <frame tileid="13" duration="100"/>
   <frame tileid="14" duration="100"/>
   <frame tileid="15" duration="100"/>
   <frame tileid="16" duration="100"/>
   <frame tileid="17" duration="100"/>
   <frame tileid="18" duration="100"/>
  </animation>
 </tile>
 <tile id="15">
  <properties>
   <property name="apex" type="bool" value="true"/>
   <property name="sfx" value="bite01"/>
  </properties>
 </tile>
 <tile id="22">
  <properties>
   <property name="action" value="attack"/>
   <property name="dir" type="int" value="2"/>
  </properties>
  <animation>
   <frame tileid="22" duration="100"/>
   <frame tileid="23" duration="100"/>
   <frame tileid="24" duration="100"/>
   <frame tileid="25" duration="100"/>
   <frame tileid="26" duration="100"/>
   <frame tileid="27" duration="100"/>
   <frame tileid="28" duration="100"/>
   <frame tileid="29" duration="100"/>
  </animation>
 </tile>
 <tile id="26">
  <properties>
   <property name="apex" type="bool" value="true"/>
   <property name="sfx" value="bite01"/>
  </properties>
 </tile>
 <tile id="33">
  <properties>
   <property name="action" value="attack"/>
   <property name="dir" type="int" value="3"/>
  </properties>
  <animation>
   <frame tileid="33" duration="100"/>
   <frame tileid="34" duration="100"/>
   <frame tileid="35" duration="100"/>
   <frame tileid="36" duration="100"/>
   <frame tileid="37" duration="100"/>
   <frame tileid="38" duration="100"/>
   <frame tileid="39" duration="100"/>
   <frame tileid="40" duration="100"/>
  </animation>
 </tile>
 <tile id="37">
  <properties>
   <property name="apex" type="bool" value="true"/>
   <property name="sfx" value="bite01"/>
  </properties>
 </tile>
 <tile id="44">
  <properties>
   <property name="action" value="attack"/>
   <property name="dir" type="int" value="4"/>
  </properties>
  <animation>
   <frame tileid="44" duration="100"/>
   <frame tileid="45" duration="100"/>
   <frame tileid="46" duration="100"/>
   <frame tileid="47" duration="100"/>
   <frame tileid="48" duration="100"/>
   <frame tileid="49" duration="100"/>
   <frame tileid="50" duration="100"/>
   <frame tileid="51" duration="100"/>
  </animation>
 </tile>
 <tile id="48">
  <properties>
   <property name="apex" type="bool" value="true"/>
   <property name="sfx" value="bite01"/>
  </properties>
 </tile>
 <tile id="55">
  <properties>
   <property name="action" value="attack"/>
   <property name="dir" type="int" value="5"/>
  </properties>
  <animation>
   <frame tileid="55" duration="100"/>
   <frame tileid="56" duration="100"/>
   <frame tileid="57" duration="100"/>
   <frame tileid="58" duration="100"/>
   <frame tileid="59" duration="100"/>
   <frame tileid="60" duration="100"/>
   <frame tileid="61" duration="100"/>
   <frame tileid="62" duration="100"/>
  </animation>
 </tile>
 <tile id="59">
  <properties>
   <property name="apex" type="bool" value="true"/>
   <property name="sfx" value="bite01"/>
  </properties>
 </tile>
 <tile id="66">
  <properties>
   <property name="action" value="attack"/>
   <property name="dir" type="int" value="6"/>
  </properties>
  <animation>
   <frame tileid="66" duration="100"/>
   <frame tileid="67" duration="100"/>
   <frame tileid="68" duration="100"/>
   <frame tileid="69" duration="100"/>
   <frame tileid="70" duration="100"/>
   <frame tileid="71" duration="100"/>
   <frame tileid="72" duration="100"/>
   <frame tileid="73" duration="100"/>
  </animation>
 </tile>
 <tile id="70">
  <properties>
   <property name="apex" type="bool" value="true"/>
   <property name="sfx" value="bite01"/>
  </properties>
 </tile>
 <tile id="77">
  <properties>
   <property name="action" value="attack"/>
   <property name="dir" type="int" value="7"/>
  </properties>
  <animation>
   <frame tileid="77" duration="100"/>
   <frame tileid="78" duration="100"/>
   <frame tileid="79" duration="100"/>
   <frame tileid="80" duration="100"/>
   <frame tileid="81" duration="100"/>
   <frame tileid="82" duration="100"/>
   <frame tileid="83" duration="100"/>
   <frame tileid="84" duration="100"/>
  </animation>
 </tile>
 <tile id="81">
  <properties>
   <property name="apex" type="bool" value="true"/>
   <property name="sfx" value="bite01"/>
  </properties>
 </tile>
 <tile id="88">
  <properties>
   <property name="action" value="death"/>
   <property name="dir" type="int" value="0"/>
   <property name="sfx" value="death01"/>
  </properties>
  <animation>
   <frame tileid="88" duration="100"/>
   <frame tileid="89" duration="100"/>
   <frame tileid="90" duration="100"/>
   <frame tileid="91" duration="100"/>
   <frame tileid="92" duration="100"/>
   <frame tileid="93" duration="100"/>
   <frame tileid="94" duration="100"/>
   <frame tileid="95" duration="100"/>
   <frame tileid="96" duration="100"/>
   <frame tileid="97" duration="100"/>
   <frame tileid="98" duration="100"/>
  </animation>
 </tile>
 <tile id="99">
  <properties>
   <property name="action" value="death"/>
   <property name="dir" type="int" value="1"/>
   <property name="sfx" value="death01"/>
  </properties>
  <animation>
   <frame tileid="99" duration="100"/>
   <frame tileid="100" duration="100"/>
   <frame tileid="101" duration="100"/>
   <frame tileid="102" duration="100"/>
   <frame tileid="103" duration="100"/>
   <frame tileid="104" duration="100"/>
   <frame tileid="105" duration="100"/>
   <frame tileid="106" duration="100"/>
   <frame tileid="107" duration="100"/>
   <frame tileid="108" duration="100"/>
   <frame tileid="109" duration="100"/>
  </animation>
 </tile>
 <tile id="110">
  <properties>
   <property name="action" value="death"/>
   <property name="dir" type="int" value="2"/>
   <property name="sfx" value="death01"/>
  </properties>
  <animation>
   <frame tileid="110" duration="100"/>
   <frame tileid="111" duration="100"/>
   <frame tileid="112" duration="100"/>
   <frame tileid="113" duration="100"/>
   <frame tileid="114" duration="100"/>
  </animation>
 </tile>
 <tile id="121">
  <properties>
   <property name="action" value="death"/>
   <property name="dir" type="int" value="3"/>
   <property name="sfx" value="death01"/>
  </properties>
  <animation>
   <frame tileid="121" duration="100"/>
   <frame tileid="122" duration="100"/>
   <frame tileid="123" duration="100"/>
   <frame tileid="124" duration="100"/>
   <frame tileid="125" duration="100"/>
  </animation>
 </tile>
 <tile id="132">
  <properties>
   <property name="action" value="death"/>
   <property name="dir" type="int" value="4"/>
   <property name="sfx" value="death01"/>
  </properties>
  <animation>
   <frame tileid="132" duration="100"/>
   <frame tileid="133" duration="100"/>
   <frame tileid="134" duration="100"/>
   <frame tileid="135" duration="100"/>
   <frame tileid="136" duration="100"/>
  </animation>
 </tile>
 <tile id="143">
  <properties>
   <property name="action" value="death"/>
   <property name="dir" type="int" value="5"/>
   <property name="sfx" value="death01"/>
  </properties>
  <animation>
   <frame tileid="143" duration="100"/>
   <frame tileid="144" duration="100"/>
   <frame tileid="145" duration="100"/>
   <frame tileid="146" duration="100"/>
   <frame tileid="147" duration="100"/>
  </animation>
 </tile>
 <tile id="154">
  <properties>
   <property name="action" value="death"/>
   <property name="dir" type="int" value="6"/>
   <property name="sfx" value="death01"/>
  </properties>
  <animation>
   <frame tileid="154" duration="100"/>
   <frame tileid="155" duration="100"/>
   <frame tileid="156" duration="100"/>
   <frame tileid="157" duration="100"/>
   <frame tileid="158" duration="100"/>
  </animation>
 </tile>
 <tile id="165">
  <properties>
   <property name="action" value="death"/>
   <property name="dir" type="int" value="7"/>
   <property name="sfx" value="death01"/>
  </properties>
  <animation>
   <frame tileid="165" duration="100"/>
   <frame tileid="166" duration="100"/>
   <frame tileid="167" duration="100"/>
   <frame tileid="168" duration="100"/>
   <frame tileid="169" duration="100"/>
  </animation>
 </tile>
 <tile id="176">
  <properties>
   <property name="action" value="idle"/>
   <property name="dir" type="int" value="0"/>
  </properties>
  <animation>
   <frame tileid="176" duration="100"/>
   <frame tileid="177" duration="100"/>
   <frame tileid="178" duration="100"/>
   <frame tileid="179" duration="100"/>
   <frame tileid="180" duration="100"/>
   <frame tileid="181" duration="100"/>
   <frame tileid="182" duration="100"/>
   <frame tileid="183" duration="100"/>
  </animation>
 </tile>
 <tile id="187">
  <properties>
   <property name="action" value="idle"/>
   <property name="dir" type="int" value="1"/>
  </properties>
  <animation>
   <frame tileid="187" duration="100"/>
   <frame tileid="188" duration="100"/>
   <frame tileid="189" duration="100"/>
   <frame tileid="190" duration="100"/>
   <frame tileid="191" duration="100"/>
   <frame tileid="192" duration="100"/>
   <frame tileid="193" duration="100"/>
   <frame tileid="194" duration="100"/>
  </animation>
 </tile>
 <tile id="198">
  <properties>
   <property name="action" value="idle"/>
   <property name="dir" type="int" value="2"/>
  </properties>
  <animation>
   <frame tileid="198" duration="100"/>
   <frame tileid="199" duration="100"/>
   <frame tileid="200" duration="100"/>
   <frame tileid="201" duration="100"/>
   <frame tileid="202" duration="100"/>
   <frame tileid="203" duration="100"/>
   <frame tileid="204" duration="100"/>
   <frame tileid="205" duration="100"/>
  </animation>
 </tile>
 <tile id="209">
  <properties>
   <property name="action" value="idle"/>
   <property name="dir" type="int" value="3"/>
  </properties>
  <animation>
   <frame tileid="209" duration="100"/>
   <frame tileid="210" duration="100"/>
   <frame tileid="211" duration="100"/>
   <frame tileid="212" duration="100"/>
   <frame tileid="213" duration="100"/>
   <frame tileid="214" duration="100"/>
   <frame tileid="215" duration="100"/>
   <frame tileid="216" duration="100"/>
  </animation>
 </tile>
 <tile id="220">
  <properties>
   <property name="action" value="idle"/>
   <property name="dir" type="int" value="4"/>
  </properties>
  <animation>
   <frame tileid="220" duration="100"/>
   <frame tileid="221" duration="100"/>
   <frame tileid="222" duration="100"/>
   <frame tileid="223" duration="100"/>
   <frame tileid="224" duration="100"/>
   <frame tileid="225" duration="100"/>
   <frame tileid="226" duration="100"/>
   <frame tileid="227" duration="100"/>
  </animation>
 </tile>
 <tile id="231">
  <properties>
   <property name="action" value="idle"/>
   <property name="dir" type="int" value="5"/>
  </properties>
  <animation>
   <frame tileid="231" duration="100"/>
   <frame tileid="232" duration="100"/>
   <frame tileid="233" duration="100"/>
   <frame tileid="234" duration="100"/>
   <frame tileid="235" duration="100"/>
   <frame tileid="236" duration="100"/>
   <frame tileid="237" duration="100"/>
   <frame tileid="238" duration="100"/>
  </animation>
 </tile>
 <tile id="242">
  <properties>
   <property name="action" value="idle"/>
   <property name="dir" type="int" value="6"/>
  </properties>
  <animation>
   <frame tileid="242" duration="100"/>
   <frame tileid="243" duration="100"/>
   <frame tileid="244" duration="100"/>
   <frame tileid="245" duration="100"/>
   <frame tileid="246" duration="100"/>
   <frame tileid="247" duration="100"/>
   <frame tileid="248" duration="100"/>
   <frame tileid="249" duration="100"/>
  </animation>
 </tile>
 <tile id="253">
  <properties>
   <property name="action" value="idle"/>
   <property name="dir" type="int" value="7"/>
  </properties>
  <animation>
   <frame tileid="253" duration="100"/>
   <frame tileid="254" duration="100"/>
   <frame tileid="255" duration="100"/>
   <frame tileid="256" duration="100"/>
   <frame tileid="257" duration="100"/>
   <frame tileid="258" duration="100"/>
   <frame tileid="259" duration="100"/>
   <frame tileid="260" duration="100"/>
  </animation>
 </tile>
 <tile id="264">
  <properties>
   <property name="action" value="run"/>
   <property name="dir" type="int" value="0"/>
  </properties>
  <animation>
   <frame tileid="264" duration="100"/>
   <frame tileid="265" duration="100"/>
   <frame tileid="266" duration="100"/>
   <frame tileid="267" duration="100"/>
  </animation>
 </tile>
 <tile id="275">
  <properties>
   <property name="action" value="run"/>
   <property name="dir" type="int" value="1"/>
  </properties>
  <animation>
   <frame tileid="275" duration="100"/>
   <frame tileid="276" duration="100"/>
   <frame tileid="277" duration="100"/>
   <frame tileid="278" duration="100"/>
  </animation>
 </tile>
 <tile id="286">
  <properties>
   <property name="action" value="run"/>
   <property name="dir" type="int" value="2"/>
  </properties>
  <animation>
   <frame tileid="286" duration="100"/>
   <frame tileid="287" duration="100"/>
   <frame tileid="288" duration="100"/>
   <frame tileid="289" duration="100"/>
  </animation>
 </tile>
 <tile id="297">
  <properties>
   <property name="action" value="run"/>
   <property name="dir" type="int" value="3"/>
  </properties>
  <animation>
   <frame tileid="297" duration="100"/>
   <frame tileid="298" duration="100"/>
   <frame tileid="299" duration="100"/>
   <frame tileid="300" duration="100"/>
  </animation>
 </tile>
 <tile id="308">
  <properties>
   <property name="action" value="run"/>
   <property name="dir" type="int" value="4"/>
  </properties>
  <animation>
   <frame tileid="308" duration="100"/>
   <frame tileid="309" duration="100"/>
   <frame tileid="310" duration="100"/>
   <frame tileid="311" duration="100"/>
  </animation>
 </tile>
 <tile id="319">
  <properties>
   <property name="action" value="run"/>
   <property name="dir" type="int" value="5"/>
  </properties>
  <animation>
   <frame tileid="319" duration="100"/>
   <frame tileid="320" duration="100"/>
   <frame tileid="321" duration="100"/>
   <frame tileid="322" duration="100"/>
  </animation>
 </tile>
 <tile id="330">
  <properties>
   <property name="action" value="run"/>
   <property name="dir" type="int" value="6"/>
  </properties>
  <animation>
   <frame tileid="330" duration="100"/>
   <frame tileid="331" duration="100"/>
   <frame tileid="332" duration="100"/>
   <frame tileid="333" duration="100"/>
  </animation>
 </tile>
 <tile id="341">
  <properties>
   <property name="action" value="run"/>
   <property name="dir" type="int" value="7"/>
  </properties>
  <animation>
   <frame tileid="341" duration="100"/>
   <frame tileid="342" duration="100"/>
   <frame tileid="343" duration="100"/>
   <frame tileid="344" duration="100"/>
  </animation>
 </tile>
 <tile id="352">
  <properties>
   <property name="action" value="walk"/>
   <property name="dir" type="int" value="0"/>
  </properties>
  <animation>
   <frame tileid="352" duration="100"/>
   <frame tileid="353" duration="100"/>
   <frame tileid="354" duration="100"/>
   <frame tileid="355" duration="100"/>
  </animation>
 </tile>
 <tile id="363">
  <properties>
   <property name="action" value="walk"/>
   <property name="dir" type="int" value="1"/>
  </properties>
  <animation>
   <frame tileid="363" duration="100"/>
   <frame tileid="364" duration="100"/>
   <frame tileid="365" duration="100"/>
   <frame tileid="366" duration="100"/>
  </animation>
 </tile>
 <tile id="374">
  <properties>
   <property name="action" value="walk"/>
   <property name="dir" type="int" value="2"/>
  </properties>
  <animation>
   <frame tileid="374" duration="100"/>
   <frame tileid="375" duration="100"/>
   <frame tileid="376" duration="100"/>
   <frame tileid="377" duration="100"/>
  </animation>
 </tile>
 <tile id="385">
  <properties>
   <property name="action" value="walk"/>
   <property name="dir" type="int" value="3"/>
  </properties>
  <animation>
   <frame tileid="385" duration="100"/>
   <frame tileid="386" duration="100"/>
   <frame tileid="387" duration="100"/>
   <frame tileid="388" duration="100"/>
  </animation>
 </tile>
 <tile id="396">
  <properties>
   <property name="action" value="walk"/>
   <property name="dir" type="int" value="4"/>
  </properties>
  <animation>
   <frame tileid="396" duration="100"/>
   <frame tileid="397" duration="100"/>
   <frame tileid="398" duration="100"/>
   <frame tileid="399" duration="100"/>
  </animation>
 </tile>
 <tile id="407">
  <properties>
   <property name="action" value="walk"/>
   <property name="dir" type="int" value="5"/>
  </properties>
  <animation>
   <frame tileid="407" duration="100"/>
   <frame tileid="408" duration="100"/>
   <frame tileid="409" duration="100"/>
   <frame tileid="410" duration="100"/>
  </animation>
 </tile>
 <tile id="418">
  <properties>
   <property name="action" value="walk"/>
   <property name="dir" type="int" value="6"/>
  </properties>
  <animation>
   <frame tileid="418" duration="100"/>
   <frame tileid="419" duration="100"/>
   <frame tileid="420" duration="100"/>
   <frame tileid="421" duration="100"/>
  </animation>
 </tile>
 <tile id="429">
  <properties>
   <property name="action" value="walk"/>
   <property name="dir" type="int" value="7"/>
  </properties>
  <animation>
   <frame tileid="429" duration="100"/>
   <frame tileid="430" duration="100"/>
   <frame tileid="431" duration="100"/>
   <frame tileid="432" duration="100"/>
  </animation>
 </tile>
</tileset>
